//! WAL op-body types (SPEC-003 §3): the MessagePack payloads that the WAL
//! entry codec carries opaquely and that recovery interprets per op. The entry
//! header already holds `seq`/`coll_id`/`op`; these are just the bodies.

use serde::{Deserialize, Serialize};

use crate::error::{Result, VecLiteError};
use crate::storage::body::StoredConfig;

/// MessagePack-encode a WAL body. `?Sized` so a slice (`&[Point]`) encodes
/// directly — it decodes back as the corresponding `Vec`.
pub(crate) fn encode<T: Serialize + ?Sized>(value: &T) -> Result<Vec<u8>> {
    rmp_serde::to_vec(value).map_err(|e| VecLiteError::Corrupt(format!("wal body encode: {e}")))
}

/// Decode a WAL body during replay; a malformed body is treated as `Corrupt`
/// (it terminates replay like a torn tail — WAL-011).
pub(crate) fn decode<T: for<'de> Deserialize<'de>>(bytes: &[u8], op: &str) -> Result<T> {
    rmp_serde::from_slice(bytes).map_err(|e| VecLiteError::Corrupt(format!("wal body {op}: {e}")))
}

/// `CREATE_COLL` body: the collection's name, aliases, and on-disk config. The
/// assigned `coll_id` rides in the WAL entry header.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CreateColl {
    pub(crate) name: String,
    pub(crate) aliases: Vec<String>,
    pub(crate) config: StoredConfig,
}

/// `RENAME` body.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Rename {
    pub(crate) new_name: String,
}

/// `ALIAS` body: create (`true`) or delete (`false`) the `alias` pointing at the
/// collection in the entry's `coll_id` (SPEC-004 §2 / SPEC-005 CORE-011).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Alias {
    pub(crate) create: bool,
    pub(crate) alias: String,
}

/// `PIDX_DECLARE` body (WAL op 8, SPEC-006 FLT-020): declare one payload index
/// at runtime. `kind` uses the PIDX segment byte (1 keyword / 2 int / 3 float,
/// SPEC-002 §3.1).
#[derive(Serialize, Deserialize)]
pub(crate) struct PidxDeclare {
    pub(crate) key: String,
    pub(crate) kind: u8,
}

// UPSERT_BATCH body = `Vec<Point>`; DELETE_BATCH body = `Vec<String>`;
// DROP_COLL body = empty. Those need no dedicated struct — encode/decode the
// value directly.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::point::Point;

    #[test]
    fn upsert_and_delete_bodies_round_trip() {
        let points = vec![
            Point::new("a", vec![1.0, 2.0]).payload(serde_json::json!({"k": 1})),
            Point::new("b", vec![3.0, 4.0]),
        ];
        let bytes = encode(&points).unwrap_or_else(|e| panic!("{e}"));
        let back: Vec<Point> = decode(&bytes, "upsert").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(back, points);

        let ids = vec!["x".to_owned(), "y".to_owned()];
        let bytes = encode(&ids).unwrap_or_else(|e| panic!("{e}"));
        let back: Vec<String> = decode(&bytes, "delete").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(back, ids);
    }

    #[test]
    fn create_coll_body_round_trips() {
        let body = CreateColl {
            name: "docs".into(),
            aliases: vec!["d".into()],
            config: StoredConfig {
                dimension: 384,
                metric: 0,
                m: 16,
                ef_construction: 200,
                ef_search: 100,
                quantization: 1,
                quant_bits: 8,
                compression: 1,
                embedding_provider: None,
                created_epoch_s: 1000,
            },
        };
        let back: CreateColl = decode(&encode(&body).unwrap_or_else(|e| panic!("{e}")), "create")
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(back, body);
    }

    #[test]
    fn garbage_body_is_corrupt() {
        let r: Result<Vec<Point>> = decode(&[0xC1, 0x00, 0xFF], "upsert");
        assert!(matches!(r, Err(VecLiteError::Corrupt(_))));
    }
}

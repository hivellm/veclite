/* tslint:disable */
/* eslint-disable */

/**
 * A collection handle (SPEC-004 §4). Cheap to clone (Arc inside).
 */
export class WasmCollection {
    private constructor();
    free(): void;
    [Symbol.dispose](): void;
    /**
     * Delete a point by id (API-022); `true` if it existed.
     */
    delete(id: string): boolean;
    /**
     * Fetch a point by id (API-021); `null` when absent.
     */
    get(id: string): any;
    /**
     * Hybrid dense+sparse search with RRF fusion (SPEC-007).
     */
    hybridSearch(options: any): any;
    /**
     * Whether the collection holds no live vectors.
     */
    isEmpty(): boolean;
    /**
     * Number of live vectors.
     */
    len(): number;
    /**
     * Force a full recompute of an auto-embed collection's vocabulary (SPEC-005).
     */
    refit(): void;
    /**
     * Cursor-based pagination over live points in stable slot order (API-022).
     */
    scroll(options: any): any;
    /**
     * k-NN search (API-030). On wasm this is an exact brute-force scan (no HNSW,
     * ADR-0002). Returns an array of `{ id, score, payload?, vector? }`.
     */
    search(query: Float32Array, options: any): any;
    /**
     * Embed `query` with the collection's provider and search (SPEC-005 §4).
     */
    searchText(query: string, options: any): any;
    /**
     * Collection statistics (FR-08/13).
     */
    stats(): any;
    /**
     * Insert-or-replace one point (API-020). `vector` is a `Float32Array`; the
     * optional `sparse` `{indices, values}` sets the hybrid lane (SPEC-007).
     */
    upsert(id: string, vector: Float32Array, payload: any, sparse: any): void;
    /**
     * Insert-or-replace a batch (API-020): an array of
     * `{ id, vector, payload?, sparse? }`.
     */
    upsertBatch(points: any): void;
    /**
     * Insert-or-replace one text document on an auto-embed collection.
     */
    upsertText(id: string, text: string, payload: any): void;
}

/**
 * A VecLite database handle (SPEC-004 §1). On wasm it is always in-memory; a
 * persistent variant is materialized by the JS wrapper via serialize/OPFS
 * (WASM-011). `VecLite` is internally `Arc`, so cloning is cheap.
 */
export class WasmDb {
    private constructor();
    free(): void;
    [Symbol.dispose](): void;
    /**
     * Get a collection handle by name or alias (CORE-051).
     */
    collection(name: string): WasmCollection;
    /**
     * Create an alias that resolves to `target` (CORE-051).
     */
    createAlias(alias: string, target: string): void;
    /**
     * Create a collection (CORE-020). `options` is a plain JS object mirroring
     * SPEC-010's `CollectionOptions` (camelCase).
     */
    createCollection(name: string, options: any): WasmCollection;
    /**
     * Delete an alias (CORE-051).
     */
    deleteAlias(alias: string): void;
    /**
     * Delete a collection (CORE-021).
     */
    deleteCollection(name: string): void;
    /**
     * The on-disk format version this build reads/writes (NFR-11).
     */
    formatVersion(): number;
    /**
     * Load a database from a `.veclite` v1 file image (WASM-010): bytes written
     * by [`WasmDb::serialize`] or by native VecLite. The JS wrapper's OPFS
     * backend and `deserialize()` entry point both route through here.
     */
    static fromBytes(bytes: Uint8Array): WasmDb;
    /**
     * List collection names.
     */
    listCollections(): string[];
    /**
     * Open an ephemeral in-memory database (FR-02).
     */
    static memory(): WasmDb;
    /**
     * Serialize the whole database to a `.veclite` v1 file image (WASM-010) as a
     * `Uint8Array` — a compacted, single-generation image that native VecLite
     * opens and [`WasmDb::from_bytes`] loads. The OPFS backend writes these bytes.
     */
    serialize(): Uint8Array;
    /**
     * Compaction is a no-op on wasm (WASM-020): the in-memory image is already
     * compacted, and `serialize()` writes the compacted form. Present so the
     * SPEC-010 surface is complete.
     */
    vacuum(): void;
}

/**
 * Split `text` into overlapping, UTF-8-safe chunks (SPEC-005 §7). Pure and
 * deterministic; `maxChars`/`overlap` default to 2048/128.
 */
export function chunk(text: string, max_chars?: number | null, overlap?: number | null): any;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly __wbg_wasmcollection_free: (a: number, b: number) => void;
    readonly __wbg_wasmdb_free: (a: number, b: number) => void;
    readonly chunk: (a: number, b: number, c: number, d: number, e: number) => void;
    readonly wasmcollection_delete: (a: number, b: number, c: number, d: number) => void;
    readonly wasmcollection_get: (a: number, b: number, c: number, d: number) => void;
    readonly wasmcollection_hybridSearch: (a: number, b: number, c: number) => void;
    readonly wasmcollection_isEmpty: (a: number) => number;
    readonly wasmcollection_len: (a: number) => number;
    readonly wasmcollection_refit: (a: number, b: number) => void;
    readonly wasmcollection_scroll: (a: number, b: number, c: number) => void;
    readonly wasmcollection_search: (a: number, b: number, c: number, d: number, e: number) => void;
    readonly wasmcollection_searchText: (a: number, b: number, c: number, d: number, e: number) => void;
    readonly wasmcollection_stats: (a: number, b: number) => void;
    readonly wasmcollection_upsert: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number) => void;
    readonly wasmcollection_upsertBatch: (a: number, b: number, c: number) => void;
    readonly wasmcollection_upsertText: (a: number, b: number, c: number, d: number, e: number, f: number, g: number) => void;
    readonly wasmdb_collection: (a: number, b: number, c: number, d: number) => void;
    readonly wasmdb_createAlias: (a: number, b: number, c: number, d: number, e: number, f: number) => void;
    readonly wasmdb_createCollection: (a: number, b: number, c: number, d: number, e: number) => void;
    readonly wasmdb_deleteAlias: (a: number, b: number, c: number, d: number) => void;
    readonly wasmdb_deleteCollection: (a: number, b: number, c: number, d: number) => void;
    readonly wasmdb_formatVersion: (a: number) => number;
    readonly wasmdb_fromBytes: (a: number, b: number, c: number) => void;
    readonly wasmdb_listCollections: (a: number, b: number) => void;
    readonly wasmdb_memory: () => number;
    readonly wasmdb_serialize: (a: number, b: number) => void;
    readonly wasmdb_vacuum: (a: number) => void;
    readonly __wbindgen_export: (a: number, b: number) => number;
    readonly __wbindgen_export2: (a: number, b: number, c: number, d: number) => number;
    readonly __wbindgen_export3: (a: number) => void;
    readonly __wbindgen_add_to_stack_pointer: (a: number) => number;
    readonly __wbindgen_export4: (a: number, b: number, c: number) => void;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;

/**
 * Instantiates the given `module`, which can either be bytes or
 * a precompiled `WebAssembly.Module`.
 *
 * @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
 *
 * @returns {InitOutput}
 */
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
 * If `module_or_path` is {RequestInfo} or {URL}, makes a request and
 * for everything else, calls `WebAssembly.instantiate` directly.
 *
 * @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
 *
 * @returns {Promise<InitOutput>}
 */
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;

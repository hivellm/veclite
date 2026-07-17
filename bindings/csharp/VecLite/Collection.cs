using System.Runtime.InteropServices;
using System.Text.Json;
using System.Text.Json.Nodes;

namespace VecLite;

/// <summary>
/// A handle to a collection (SPEC-011). A lightweight view over the database; its
/// native handle is released on Dispose (CS-010). Safe for concurrent use.
/// </summary>
public sealed unsafe class Collection : IDisposable
{
    private readonly Database _db; // keeps the parent db alive while in use
    private readonly VecLiteCollectionHandle _handle;

    private Collection(Database db, VecLiteCollectionHandle handle)
    {
        _db = db;
        _handle = handle;
    }

    internal static Collection Wrap(Database db, IntPtr raw)
    {
        var handle = new VecLiteCollectionHandle();
        Marshal.InitHandle(handle, raw);
        return new Collection(db, handle);
    }

    private IntPtr Raw => _handle.DangerousGetHandle();

    public void Dispose() => _handle.Dispose();

    private void Done() { GC.KeepAlive(this); GC.KeepAlive(_db); }

    /// <summary>Insert or replace one point. Uses zero-copy pinned vector interop
    /// (CS-011). A sparse lane routes through the batch path (the single-upsert C
    /// entry point takes only a dense vector + payload).</summary>
    public void Upsert(string id, ReadOnlySpan<float> vector, object? payload = null, SparseVector? sparse = null)
    {
        if (sparse != null)
        {
            UpsertBatch(new[] { new BatchPoint(id, vector.ToArray(), payload, sparse) });
            return;
        }
        var idBytes = Interop.ToUtf8Nul(id);
        byte[]? payloadBytes = payload == null ? null : Interop.JsonBytes(payload);
        fixed (byte* i = idBytes)
        fixed (float* v = vector)
        fixed (byte* pl = payloadBytes)
            Interop.Check(Native.vl_upsert(Raw, i, v, (nuint)vector.Length, pl, (nuint)(payloadBytes?.Length ?? 0), Native.CodecJson));
        Done();
    }

    /// <summary>A point for UpsertBatch: id + vector + optional payload/sparse.</summary>
    public readonly record struct BatchPoint(string Id, float[] Vector, object? Payload = null, SparseVector? Sparse = null);

    /// <summary>Insert or replace many points atomically.</summary>
    public void UpsertBatch(IEnumerable<BatchPoint> points)
    {
        var wire = points.Select(p => new Dictionary<string, object?>
        {
            ["id"] = p.Id,
            ["vector"] = p.Vector,
            ["payload"] = p.Payload,
            ["sparse"] = p.Sparse,
        }).ToList();
        var blob = Interop.JsonBytes(wire);
        fixed (byte* b = blob)
            Interop.Check(Native.vl_upsert_batch(Raw, b, (nuint)blob.Length, Native.CodecJson));
        Done();
    }

    /// <summary>Insert or replace one text document (auto-embed collections).</summary>
    public void UpsertText(string id, string text, object? payload = null)
    {
        var idBytes = Interop.ToUtf8Nul(id);
        var textBytes = Interop.ToUtf8Nul(text);
        byte[]? payloadBytes = payload == null ? null : Interop.JsonBytes(payload);
        fixed (byte* i = idBytes)
        fixed (byte* t = textBytes)
        fixed (byte* pl = payloadBytes)
            Interop.Check(Native.vl_upsert_text(Raw, i, t, pl, (nuint)(payloadBytes?.Length ?? 0), Native.CodecJson));
        Done();
    }

    /// <summary>Delete one id; returns whether it existed.</summary>
    public bool Delete(string id)
    {
        var idBytes = Interop.ToUtf8Nul(id);
        byte existed;
        fixed (byte* i = idBytes)
            Interop.Check(Native.vl_delete(Raw, i, out existed));
        Done();
        return existed != 0;
    }

    /// <summary>Delete many ids; returns how many existed.</summary>
    public int DeleteBatch(IReadOnlyList<string> ids)
    {
        var blob = Interop.JsonBytes(ids);
        ulong deleted;
        fixed (byte* b = blob)
            Interop.Check(Native.vl_delete_batch(Raw, b, (nuint)blob.Length, Native.CodecJson, out deleted));
        Done();
        return (int)deleted;
    }

    /// <summary>Number of live vectors.</summary>
    public long Count()
    {
        Interop.Check(Native.vl_count(Raw, out ulong n));
        Done();
        return (long)n;
    }

    /// <summary>Fetch one point by id, or null if absent.</summary>
    public Point? Get(string id)
    {
        var idBytes = Interop.ToUtf8Nul(id);
        Native.VlBuf buf;
        fixed (byte* i = idBytes)
            Interop.Check(Native.vl_get(Raw, i, Native.CodecJson, out buf));
        Done();
        var raw = Interop.TakeBuf(ref buf);
        if (raw.Length == 0)
            return null;
        var node = JsonNode.Parse(raw)!;
        return new Point
        {
            Id = node["id"]!.GetValue<string>(),
            Vector = ToFloats(node["vector"]),
            Payload = node["payload"]?.DeepClone(),
        };
    }

    /// <summary>k-NN search over a dense query vector (CS-011 pinned interop).</summary>
    public IReadOnlyList<Hit> Search(ReadOnlySpan<float> query, SearchOptions? options = null)
    {
        options ??= new SearchOptions();
        byte[]? optBytes = QueryOptsBytes(options);
        IntPtr hits;
        fixed (float* v = query)
        fixed (byte* o = optBytes)
            Interop.Check(Native.vl_search(Raw, v, (nuint)query.Length, (uint)Math.Max(options.Limit, 1), o, (nuint)(optBytes?.Length ?? 0), Native.CodecJson, out hits));
        Done();
        return CollectHits(hits);
    }

    /// <summary>Text search (auto-embed collections).</summary>
    public IReadOnlyList<Hit> SearchText(string query, SearchOptions? options = null)
    {
        options ??= new SearchOptions();
        var qBytes = Interop.ToUtf8Nul(query);
        byte[]? optBytes = QueryOptsBytes(options);
        IntPtr hits;
        fixed (byte* q = qBytes)
        fixed (byte* o = optBytes)
            Interop.Check(Native.vl_search_text(Raw, q, (uint)Math.Max(options.Limit, 1), o, (nuint)(optBytes?.Length ?? 0), Native.CodecJson, out hits));
        Done();
        return CollectHits(hits);
    }

    /// <summary>Fused dense+sparse+text search (at least one channel).</summary>
    public IReadOnlyList<Hit> HybridSearch(HybridOptions options)
    {
        var wire = new Dictionary<string, object?>
        {
            ["dense"] = options.Dense,
            ["text"] = string.IsNullOrEmpty(options.Text) ? null : options.Text,
            ["sparse"] = options.Sparse,
            ["limit"] = options.Limit,
            ["alpha"] = options.Alpha,
            ["rrf_k"] = options.RrfK,
            ["with_payload"] = options.WithPayload,
            ["with_vector"] = options.WithVector,
            ["filter"] = options.Filter,
        };
        var blob = Interop.JsonBytes(wire);
        IntPtr hits;
        fixed (byte* b = blob)
            Interop.Check(Native.vl_hybrid_search(Raw, b, (nuint)blob.Length, Native.CodecJson, out hits));
        Done();
        return CollectHits(hits);
    }

    /// <summary>Scroll the collection in id order.</summary>
    public Page Scroll(ScrollOptions? options = null)
    {
        options ??= new ScrollOptions();
        var wire = new Dictionary<string, object?>
        {
            ["limit"] = options.Limit,
            ["cursor"] = options.OffsetId,
            ["filter"] = options.Filter,
        };
        var blob = Interop.JsonBytes(wire);
        IntPtr page;
        fixed (byte* b = blob)
            Interop.Check(Native.vl_scroll(Raw, b, (nuint)blob.Length, Native.CodecJson, out page));
        Done();
        try
        {
            int count = (int)Native.vl_page_len(page);
            var pts = new List<Point>(count);
            for (uint i = 0; i < count; i++)
            {
                Interop.Check(Native.vl_page_point(page, i, out var buf));
                var raw = Interop.TakeBuf(ref buf);
                var node = JsonNode.Parse(raw)!;
                pts.Add(new Point { Id = node["id"]!.GetValue<string>(), Vector = ToFloats(node["vector"]), Payload = node["payload"]?.DeepClone() });
            }
            string? cursor = Interop.PtrToString(Native.vl_page_cursor(page));
            return new Page { Points = pts, NextCursor = cursor };
        }
        finally
        {
            Native.vl_page_free(page);
        }
    }

    /// <summary>Rebuild the ANN index from the live vectors.</summary>
    public void Reindex() { Interop.Check(Native.vl_collection_reindex(Raw)); Done(); }

    /// <summary>Recompute the text embedder's vocabulary and re-embed documents.</summary>
    public void Refit() { Interop.Check(Native.vl_collection_refit(Raw)); Done(); }

    /// <summary>Declare a payload index on a field.</summary>
    public void CreatePayloadIndex(string field, PayloadIndexKind kind)
    {
        var fb = Interop.ToUtf8Nul(field);
        fixed (byte* f = fb)
            Interop.Check(Native.vl_payload_index_create(Raw, f, (byte)kind));
        Done();
    }

    /// <summary>Collection statistics.</summary>
    public JsonNode Stats()
    {
        Interop.Check(Native.vl_collection_stats(Raw, Native.CodecJson, out var buf));
        Done();
        var raw = Interop.TakeBuf(ref buf);
        return JsonNode.Parse(raw) ?? new JsonObject();
    }

    // ── helpers ──────────────────────────────────────────────────────────────

    private static byte[]? QueryOptsBytes(SearchOptions o)
    {
        if (o.EfSearch == null && o.WithPayload == null && o.WithVector == null && o.Filter == null)
            return null;
        var wire = new Dictionary<string, object?>
        {
            ["ef_search"] = o.EfSearch,
            ["with_payload"] = o.WithPayload,
            ["with_vector"] = o.WithVector,
            ["filter"] = o.Filter,
        };
        return Interop.JsonBytes(wire);
    }

    private static float[] ToFloats(JsonNode? node)
    {
        if (node is not JsonArray arr)
            return Array.Empty<float>();
        var v = new float[arr.Count];
        for (int i = 0; i < arr.Count; i++)
            v[i] = arr[i]!.GetValue<float>();
        return v;
    }

    private static List<Hit> CollectHits(IntPtr hits)
    {
        try
        {
            int n = (int)Native.vl_hits_len(hits);
            var list = new List<Hit>(n);
            for (uint i = 0; i < n; i++)
            {
                if (Native.vl_hits_get(hits, i, out var view) != Native.VL_OK)
                    continue;
                JsonNode? payload = null;
                if (view.payload != null && view.payload_len > 0)
                {
                    var bytes = new byte[(int)view.payload_len];
                    Marshal.Copy((IntPtr)view.payload, bytes, 0, bytes.Length);
                    payload = JsonNode.Parse(bytes);
                }
                float[]? vec = null;
                if (view.has_vector != 0 && view.vector != null && view.vector_len > 0)
                {
                    vec = new float[(int)view.vector_len];
                    Marshal.Copy((IntPtr)view.vector, vec, 0, vec.Length);
                }
                list.Add(new Hit { Id = Interop.PtrToString(view.id) ?? "", Score = view.score, Payload = payload, Vector = vec });
            }
            return list;
        }
        finally
        {
            Native.vl_hits_free(hits);
        }
    }
}

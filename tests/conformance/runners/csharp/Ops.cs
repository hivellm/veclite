using System.Text.Json.Nodes;
using VecLite;

/// <summary>Op dispatch producing the shared canonical observation shapes.</summary>
internal static class Ops
{
    public static JsonObject Execute(Database db, string op, Dictionary<string, object?> a)
    {
        try
        {
            return Dispatch(db, op, a);
        }
        catch (VecLiteException e)
        {
            return new JsonObject { ["error"] = e.CodeString };
        }
    }

    private static JsonObject Dispatch(Database db, string op, Dictionary<string, object?> a)
    {
        Collection Coll() => db.GetCollection(Coerce.Str(a.GetValueOrDefault("collection")));

        switch (op)
        {
            case "create_collection":
                {
                    var opts = new CollectionOptions
                    {
                        Dimension = Coerce.Int(a.GetValueOrDefault("dimension")),
                        Metric = MetricOf(Coerce.StrOrNull(a.GetValueOrDefault("metric"))),
                    };
                    if (a.TryGetValue("quantization_bits", out var q) && q != null) opts.QuantizationBits = (byte)Coerce.Int(q);
                    if (a.TryGetValue("auto_embed", out var ae) && ae != null) opts.EmbeddingProvider = Coerce.Str(ae);
                    db.CreateCollection(Coerce.Str(a.GetValueOrDefault("name")), opts).Dispose();
                    return new JsonObject();
                }
            case "delete_collection":
                db.DropCollection(Coerce.Str(a.GetValueOrDefault("name")));
                return new JsonObject();
            case "list_collections":
                return new JsonObject { ["ids"] = Arr(db.ListCollections()) };
            case "create_alias":
                db.CreateAlias(Coerce.Str(a.GetValueOrDefault("alias")), Coerce.Str(a.GetValueOrDefault("target")));
                return new JsonObject();
            case "delete_alias":
                db.DeleteAlias(Coerce.Str(a.GetValueOrDefault("alias")));
                return new JsonObject();
            case "upsert":
                {
                    using var c = Coll();
                    c.Upsert(Coerce.Str(a.GetValueOrDefault("id")), Coerce.Floats(a.GetValueOrDefault("vector")),
                        Coerce.ToNode(a.GetValueOrDefault("payload")), Coerce.Sparse(a.GetValueOrDefault("sparse")));
                    return new JsonObject();
                }
            case "upsert_batch":
                {
                    using var c = Coll();
                    var pts = ((List<object>)a["points"]!).Select(p =>
                    {
                        var m = Coerce.Dict(p);
                        return new Collection.BatchPoint(Coerce.Str(m.GetValueOrDefault("id")), Coerce.Floats(m.GetValueOrDefault("vector")),
                            Coerce.ToNode(m.GetValueOrDefault("payload")), Coerce.Sparse(m.GetValueOrDefault("sparse")));
                    });
                    c.UpsertBatch(pts);
                    return new JsonObject();
                }
            case "upsert_text":
                {
                    using var c = Coll();
                    c.UpsertText(Coerce.Str(a.GetValueOrDefault("id")), Coerce.Str(a.GetValueOrDefault("text")), Coerce.ToNode(a.GetValueOrDefault("payload")));
                    return new JsonObject();
                }
            case "refit":
                {
                    using var c = Coll();
                    c.Refit();
                    return new JsonObject();
                }
            case "get":
                {
                    using var c = Coll();
                    var p = c.Get(Coerce.Str(a.GetValueOrDefault("id")));
                    if (p == null) return new JsonObject { ["result"] = null };
                    return new JsonObject { ["result"] = new JsonObject { ["id"] = p.Id, ["vector"] = FloatArr(p.Vector), ["payload"] = p.Payload?.DeepClone() } };
                }
            case "delete":
                {
                    using var c = Coll();
                    return new JsonObject { ["value"] = c.Delete(Coerce.Str(a.GetValueOrDefault("id"))) };
                }
            case "len":
                {
                    using var c = Coll();
                    return new JsonObject { ["value"] = c.Count() };
                }
            case "stats":
                {
                    using var c = Coll();
                    var s = c.Stats();
                    return new JsonObject
                    {
                        ["value"] = new JsonObject
                        {
                            ["dimension"] = s["dimension"]?.DeepClone(),
                            ["len"] = s["len"]?.DeepClone(),
                            ["tombstones"] = s["tombstones"]?.DeepClone(),
                            ["auto_embed"] = s["auto_embed"]?.DeepClone(),
                        }
                    };
                }
            case "search":
                {
                    using var c = Coll();
                    return HitsObs(c.Search(Coerce.Floats(a.GetValueOrDefault("vector")), SearchOpts(a)));
                }
            case "search_text":
                {
                    using var c = Coll();
                    return HitsObs(c.SearchText(Coerce.Str(a.GetValueOrDefault("query")), new SearchOptions { Limit = Coerce.Int(a.GetValueOrDefault("limit"), 10) }));
                }
            case "hybrid_search":
                {
                    using var c = Coll();
                    return HitsObs(c.HybridSearch(HybridOpts(a)));
                }
            case "scroll":
                {
                    using var c = Coll();
                    var page = c.Scroll(new ScrollOptions { Limit = Coerce.Int(a.GetValueOrDefault("limit")), OffsetId = Coerce.StrOrNull(a.GetValueOrDefault("offset_id")), Filter = Coerce.ToNode(a.GetValueOrDefault("filter")) });
                    var ids = new JsonArray();
                    foreach (var p in page.Points) ids.Add(p.Id);
                    return new JsonObject { ["ids"] = ids, ["next_cursor"] = page.NextCursor };
                }
            case "chunk":
                {
                    var chunks = Chunker.Chunk(Coerce.Str(a.GetValueOrDefault("text")), Coerce.Int(a.GetValueOrDefault("max_chars")), Coerce.Int(a.GetValueOrDefault("overlap")));
                    var arr = new JsonArray();
                    foreach (var ch in chunks) arr.Add(new JsonObject { ["text"] = ch.Text, ["start"] = ch.Start, ["end"] = ch.End });
                    return new JsonObject { ["result"] = arr };
                }
            default:
                throw new VecLiteException(ErrorCode.Internal, "unknown op " + op);
        }
    }

    private static Metric MetricOf(string? m) => m switch
    {
        "euclidean" or "l2" => Metric.Euclidean,
        "dot" or "dotproduct" or "dot_product" => Metric.DotProduct,
        _ => Metric.Cosine,
    };

    private static SearchOptions SearchOpts(Dictionary<string, object?> a)
    {
        var o = new SearchOptions { Limit = Coerce.Int(a.GetValueOrDefault("limit"), 10), Filter = Coerce.ToNode(a.GetValueOrDefault("filter")) };
        if (a.TryGetValue("ef_search", out var ef) && ef != null) o.EfSearch = Coerce.Int(ef);
        if (a.TryGetValue("with_payload", out var wp) && wp != null) o.WithPayload = Coerce.Bool(wp);
        if (a.TryGetValue("with_vector", out var wv) && wv != null) o.WithVector = Coerce.Bool(wv);
        return o;
    }

    private static HybridOptions HybridOpts(Dictionary<string, object?> a)
    {
        var o = new HybridOptions { Limit = Coerce.Int(a.GetValueOrDefault("limit"), 10), Text = Coerce.StrOrNull(a.GetValueOrDefault("text")), Filter = Coerce.ToNode(a.GetValueOrDefault("filter")) };
        if (a.TryGetValue("dense", out var d) && d != null) o.Dense = Coerce.Floats(d);
        o.Sparse = Coerce.Sparse(a.GetValueOrDefault("sparse"));
        if (a.TryGetValue("alpha", out var al) && al != null) o.Alpha = (float)Coerce.Double(al);
        if (a.TryGetValue("rrf_k", out var rk) && rk != null) o.RrfK = (float)Coerce.Double(rk);
        return o;
    }

    private static JsonObject HitsObs(IReadOnlyList<Hit> hits)
    {
        var ids = new JsonArray();
        var scores = new JsonArray();
        foreach (var h in hits) { ids.Add(h.Id); scores.Add((double)h.Score); }
        return new JsonObject { ["ids"] = ids, ["scores"] = scores };
    }

    private static JsonArray Arr(IEnumerable<string> items)
    {
        var arr = new JsonArray();
        foreach (var s in items) arr.Add(s);
        return arr;
    }

    private static JsonArray FloatArr(float[] v)
    {
        var arr = new JsonArray();
        foreach (var f in v) arr.Add((double)f);
        return arr;
    }
}

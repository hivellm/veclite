// VecLite C# quickstart (SPEC-011). Doubles as the clean-machine install proof
// (REL-020): it uses the VecLite package's prebuilt native asset — no Rust
// toolchain — and exercises the core flow. Exits non-zero on any surprise.
//
// Run: `dotnet run --project bindings/csharp/Quickstart`

using System.Text.Json.Nodes;
using VecLite;

// A durable single-file database — no server, no config (FR-01/02).
var path = Path.Combine(Path.GetTempPath(), $"veclite-cs-{Environment.ProcessId}.veclite");
using var db = Database.Open(path);

// BYO-vector collection, cosine metric, full precision.
using var docs = db.CreateCollection("docs", new CollectionOptions
{
    Dimension = 3,
    Metric = Metric.Cosine,
    QuantizationBits = 0,
});
docs.Upsert("a", new float[] { 1, 0, 0 }, new { lang = "en" });
docs.Upsert("b", new float[] { 0, 1, 0 }, new { lang = "fr" });
docs.Upsert("c", new float[] { 0.9f, 0.1f, 0f }, new { lang = "en" });

// k-NN search with a payload filter (SPEC-006): only the English vectors.
var hits = docs.Search(new float[] { 1, 0, 0 }, new SearchOptions
{
    Limit = 2,
    Filter = JsonNode.Parse("""{"must":[{"key":"lang","match":{"value":"en"}}]}"""),
});
var ids = hits.Select(h => h.Id).ToArray();
if (!ids.SequenceEqual(new[] { "a", "c" }))
{
    Console.Error.WriteLine($"unexpected ids: {string.Join(", ", ids)}");
    return 1;
}

// An auto-embed (BM25) collection: text in, ranked ids out (SPEC-005).
using var notes = db.CreateCollection("notes", new CollectionOptions
{
    Dimension = 128,
    EmbeddingProvider = "bm25",
});
notes.UpsertText("n1", "the quick brown fox");
notes.UpsertText("n2", "a lazy sleeping dog");
if (notes.SearchText("quick fox", new SearchOptions { Limit = 2 }).Count == 0)
{
    Console.Error.WriteLine("text search returned nothing");
    return 1;
}

File.Delete(path);
Console.WriteLine($"veclite {Database.Version()}: quickstart OK ({string.Join(", ", ids)})");
return 0;

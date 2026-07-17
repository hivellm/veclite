using System.Text.Json.Nodes;
using VecLite;
using Xunit;

namespace VecLite.Tests;

public class BindingTests
{
    [Fact]
    public void Quickstart_Memory()
    {
        using var db = Database.Memory();
        using var docs = db.CreateCollection("docs", new CollectionOptions { Dimension = 3, Metric = Metric.Euclidean, QuantizationBits = 0 });

        docs.Upsert("a", new float[] { 1, 0, 0 }, new { lang = "en" });
        docs.Upsert("b", new float[] { 0, 1, 0 });
        Assert.Equal(2, docs.Count());

        var hits = docs.Search(new float[] { 0.9f, 0.1f, 0f }, new SearchOptions { Limit = 1 });
        Assert.Single(hits);
        Assert.Equal("a", hits[0].Id);
        Assert.Equal("en", hits[0].Payload!["lang"]!.GetValue<string>());

        var got = docs.Get("a");
        Assert.NotNull(got);
        Assert.Equal("a", got!.Id);
        Assert.Null(docs.Get("missing"));
        Assert.True(docs.Delete("a"));
    }

    [Fact]
    public void FilteredSearch_And_Scroll()
    {
        using var db = Database.Memory();
        using var c = db.CreateCollection("v", new CollectionOptions { Dimension = 2, Metric = Metric.Euclidean, QuantizationBits = 0 });
        c.UpsertBatch(new[]
        {
            new Collection.BatchPoint("a", new float[] { 0, 0 }, new { lang = "en" }),
            new Collection.BatchPoint("b", new float[] { 1, 0 }, new { lang = "pt" }),
            new Collection.BatchPoint("c", new float[] { 0, 1 }, new { lang = "en" }),
        });
        Assert.Equal(3, c.Count());

        var filter = JsonNode.Parse("""{"must":[{"key":"lang","match":{"value":"en"}}]}""");
        var hits = c.Search(new float[] { 0, 0 }, new SearchOptions { Limit = 10, WithVector = true, Filter = filter });
        Assert.Equal(2, hits.Count);
        Assert.Equal(2, hits[0].Vector!.Length);

        var seen = new HashSet<string>();
        string? cursor = null;
        do
        {
            var page = c.Scroll(new ScrollOptions { Limit = 2, OffsetId = cursor });
            foreach (var p in page.Points) seen.Add(p.Id);
            cursor = page.NextCursor;
        } while (cursor != null);
        Assert.Equal(3, seen.Count);
    }

    [Fact]
    public void Errors_Map_To_Typed_Exceptions()
    {
        using var db = Database.Memory();
        using var c = db.CreateCollection("v", new CollectionOptions { Dimension = 3, Metric = Metric.Euclidean, QuantizationBits = 0 });

        Assert.Throws<AlreadyExistsException>(() => db.CreateCollection("v", new CollectionOptions { Dimension = 3 }));
        Assert.Throws<CollectionNotFoundException>(() => db.GetCollection("nope"));

        var ex = Assert.Throws<DimensionMismatchException>(() => c.Upsert("x", new float[] { 1, 2 }));
        Assert.Equal(ErrorCode.DimensionMismatch, ex.Code);
        Assert.Equal("DIMENSION_MISMATCH", ex.CodeString);

        var prov = Assert.Throws<VecLiteException>(() => db.CreateCollection("bad", new CollectionOptions { Dimension = 8, EmbeddingProvider = "no-such" }));
        Assert.Equal(ErrorCode.UnsupportedProvider, prov.Code);
    }

    [Fact]
    public void Locked_Is_Reported()
    {
        var path = Path.Combine(Path.GetTempPath(), $"veclite-cs-{Guid.NewGuid():N}.veclite");
        try
        {
            using var db = Database.Open(path);
            var ex = Assert.Throws<LockedException>(() => Database.Open(path));
            Assert.Equal(ErrorCode.Locked, ex.Code);
        }
        finally
        {
            File.Delete(path);
        }
    }

    [Fact]
    public void Concurrent_Access_Is_Safe()
    {
        using var db = Database.Memory();
        using var c = db.CreateCollection("v", new CollectionOptions { Dimension = 4, Metric = Metric.Cosine, QuantizationBits = 0 });
        const int workers = 16, per = 64;

        Parallel.For(0, workers, w =>
        {
            for (int i = 0; i < per; i++)
            {
                var vec = new float[] { w, i, w ^ i, 1 };
                c.Upsert($"w{w}_{i}", vec);
                _ = c.Search(vec, new SearchOptions { Limit = 5 });
            }
        });

        Assert.Equal((long)workers * per, c.Count());
    }

    [Fact]
    public void SafeHandle_Releases_Lock_Under_GC()
    {
        var path = Path.Combine(Path.GetTempPath(), $"veclite-cs-gc-{Guid.NewGuid():N}.veclite");
        try
        {
            // Open many file handles without Dispose; the SafeHandle finalizer
            // must release each lock so the path stays reopenable (CS-010).
            for (int i = 0; i < 20; i++)
            {
                LeakOne(path);
                GC.Collect();
                GC.WaitForPendingFinalizers();
            }
            using var ok = Database.Open(path); // succeeds only if locks were released
            Assert.True(Database.AbiVersion() >= 1);
        }
        finally
        {
            File.Delete(path);
        }
    }

    private static void LeakOne(string path)
    {
        var db = Database.Open(path);
        _ = db.ListCollections();
        // no Dispose — drop the reference and let finalization release it
    }
}

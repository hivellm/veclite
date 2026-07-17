using System.Text.Json.Nodes;
using System.Text.Json.Serialization;

namespace VecLite;

/// <summary>Distance metric (SPEC-004).</summary>
public enum Metric
{
    Cosine,
    Euclidean,
    DotProduct,
}

/// <summary>Payload-index kinds for CreatePayloadIndex.</summary>
public enum PayloadIndexKind : byte
{
    Keyword = 0,
    Integer = 1,
    Float = 2,
}

internal static class MetricExtensions
{
    internal static string Wire(this Metric m) => m switch
    {
        Metric.Euclidean => "euclidean",
        Metric.DotProduct => "dot",
        _ => "cosine",
    };
}

/// <summary>Options for Open. ReadOnly is accepted for forward compatibility;
/// the current C ABI opens with defaults.</summary>
public sealed class OpenOptions
{
    public bool ReadOnly { get; set; }
}

/// <summary>Options for CreateCollection.</summary>
public sealed class CollectionOptions
{
    public int Dimension { get; set; }
    public Metric Metric { get; set; } = Metric.Cosine;
    public byte? QuantizationBits { get; set; }
    public string? EmbeddingProvider { get; set; }
}

/// <summary>A sparse lane for hybrid search (SPEC-007).</summary>
public sealed class SparseVector
{
    [JsonPropertyName("indices")] public uint[] Indices { get; set; } = Array.Empty<uint>();
    [JsonPropertyName("values")] public float[] Values { get; set; } = Array.Empty<float>();
}

/// <summary>Options for k-NN / text search.</summary>
public sealed class SearchOptions
{
    public int Limit { get; set; } = 10;
    public int? EfSearch { get; set; }
    public bool? WithPayload { get; set; }
    public bool? WithVector { get; set; }
    public JsonNode? Filter { get; set; }
}

/// <summary>Options for a fused hybrid search (at least one channel).</summary>
public sealed class HybridOptions
{
    public float[]? Dense { get; set; }
    public string? Text { get; set; }
    public SparseVector? Sparse { get; set; }
    public int Limit { get; set; } = 10;
    public float? Alpha { get; set; }
    public float? RrfK { get; set; }
    public bool? WithPayload { get; set; }
    public bool? WithVector { get; set; }
    public JsonNode? Filter { get; set; }
}

/// <summary>Options for scroll pagination.</summary>
public sealed class ScrollOptions
{
    public int Limit { get; set; } = 100;
    public string? OffsetId { get; set; }
    public JsonNode? Filter { get; set; }
}

/// <summary>One ranked search result.</summary>
public sealed class Hit
{
    public string Id { get; init; } = "";
    public float Score { get; init; }
    public JsonNode? Payload { get; init; }
    public float[]? Vector { get; init; }
}

/// <summary>A stored point.</summary>
public sealed class Point
{
    public string Id { get; init; } = "";
    public float[] Vector { get; init; } = Array.Empty<float>();
    public JsonNode? Payload { get; init; }
}

/// <summary>A scroll page.</summary>
public sealed class Page
{
    public IReadOnlyList<Point> Points { get; init; } = Array.Empty<Point>();
    public string? NextCursor { get; init; }
}

/// <summary>One text chunk with its byte range in the source.</summary>
public sealed class TextChunk
{
    public string Text { get; init; } = "";
    public int Start { get; init; }
    public int End { get; init; }
}

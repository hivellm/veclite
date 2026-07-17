using System.Globalization;
using System.Text.Json.Nodes;

/// <summary>
/// YamlDotNet deserializes every scalar as a string; these helpers coerce the
/// object graph into typed values and JSON nodes (inferring bool/number/string
/// from the text, matching how the golden JSON types them).
/// </summary>
internal static class Coerce
{
    public static Dictionary<string, object?> Dict(object? o)
    {
        var result = new Dictionary<string, object?>();
        if (o is Dictionary<object, object> d)
            foreach (var (k, v) in d)
                result[k.ToString()!] = v;
        return result;
    }

    public static string Str(object? o) => o?.ToString() ?? "";

    public static string? StrOrNull(object? o) => o?.ToString();

    public static int Int(object? o, int def = 0) =>
        o == null ? def : (int)Math.Round(Double(o));

    public static double Double(object? o)
    {
        if (o == null) return 0;
        return double.TryParse(o.ToString(), NumberStyles.Any, CultureInfo.InvariantCulture, out var d) ? d : 0;
    }

    public static bool Bool(object? o) => o != null && bool.TryParse(o.ToString(), out var b) && b;

    public static float[] Floats(object? o)
    {
        if (o is not List<object> list) return Array.Empty<float>();
        return list.Select(x => (float)Double(x)).ToArray();
    }

    public static uint[] UInts(object? o)
    {
        if (o is not List<object> list) return Array.Empty<uint>();
        return list.Select(x => (uint)Int(x)).ToArray();
    }

    public static VecLite.SparseVector? Sparse(object? o)
    {
        if (o is not Dictionary<object, object> d) return null;
        var m = Dict(d);
        return new VecLite.SparseVector { Indices = UInts(m.GetValueOrDefault("indices")), Values = Floats(m.GetValueOrDefault("values")) };
    }

    /// <summary>Recursively convert a YAML object graph to a JsonNode, inferring
    /// scalar types (used for `expect` comparisons against typed observations).</summary>
    public static JsonNode? ToNode(object? o)
    {
        switch (o)
        {
            case null:
                return null;
            case Dictionary<object, object> d:
                {
                    var obj = new JsonObject();
                    foreach (var (k, v) in d)
                        obj[k.ToString()!] = ToNode(v);
                    return obj;
                }
            case List<object> list:
                {
                    var arr = new JsonArray();
                    foreach (var v in list)
                        arr.Add(ToNode(v));
                    return arr;
                }
            default:
                return InferScalar(o.ToString()!);
        }
    }

    private static JsonNode InferScalar(string s)
    {
        if (bool.TryParse(s, out var b)) return JsonValue.Create(b);
        if (long.TryParse(s, NumberStyles.Integer, CultureInfo.InvariantCulture, out var l)) return JsonValue.Create(l);
        if (double.TryParse(s, NumberStyles.Any, CultureInfo.InvariantCulture, out var dbl)) return JsonValue.Create(dbl);
        return JsonValue.Create(s);
    }
}

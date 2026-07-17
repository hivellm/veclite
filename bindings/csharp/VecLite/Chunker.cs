using System.Text.Json;

namespace VecLite;

/// <summary>Pure text chunking (SPEC-005 §7).</summary>
public static unsafe class Chunker
{
    /// <summary>Split text into overlapping, UTF-8-safe chunks. maxChars/overlap
    /// default to 2048/128 when zero.</summary>
    public static IReadOnlyList<TextChunk> Chunk(string text, int maxChars = 0, int overlap = 0)
    {
        Native.EnsureInitialized();
        var opts = new Dictionary<string, int>();
        if (maxChars > 0) opts["max_chars"] = maxChars;
        if (overlap > 0) opts["overlap"] = overlap;
        var textBytes = Interop.ToUtf8Nul(text);
        var optBytes = Interop.JsonBytes(opts);
        Native.VlBuf buf;
        fixed (byte* t = textBytes)
        fixed (byte* o = optBytes)
            Interop.Check(Native.vl_chunk(t, o, (nuint)optBytes.Length, Native.CodecJson, out buf));
        var raw = Interop.TakeBuf(ref buf);
        return JsonSerializer.Deserialize<List<TextChunk>>(raw, Interop.Json) ?? new List<TextChunk>();
    }
}

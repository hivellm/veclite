using System.Runtime.InteropServices;
using System.Text;
using System.Text.Json;

namespace VecLite;

/// <summary>Marshaling + error-check helpers shared by the binding.</summary>
internal static unsafe class Interop
{
    internal static readonly JsonSerializerOptions Json = new()
    {
        DefaultIgnoreCondition = System.Text.Json.Serialization.JsonIgnoreCondition.WhenWritingNull,
        PropertyNameCaseInsensitive = true,
    };

    /// <summary>Throw the matching VecLiteException if code is not VL_OK.</summary>
    internal static void Check(int code)
    {
        if (code == Native.VL_OK)
            return;
        throw VecLiteException.FromCode(code, LastMessage());
    }

    internal static string LastMessage() => PtrToString(Native.vl_last_error_message()) ?? "veclite: error";

    internal static string CodeString(ErrorCode code) => code switch
    {
        ErrorCode.CollectionNotFound => "COLLECTION_NOT_FOUND",
        ErrorCode.VectorNotFound => "VECTOR_NOT_FOUND",
        ErrorCode.AlreadyExists => "ALREADY_EXISTS",
        ErrorCode.DimensionMismatch => "DIMENSION_MISMATCH",
        ErrorCode.Locked => "LOCKED",
        ErrorCode.Corrupt => "CORRUPT",
        ErrorCode.UnsupportedFormat => "UNSUPPORTED_FORMAT",
        ErrorCode.UnsupportedProvider => "UNSUPPORTED_PROVIDER",
        ErrorCode.ReadOnly => "READ_ONLY",
        ErrorCode.InvalidArgument => "INVALID_ARGUMENT",
        ErrorCode.Io => "IO",
        ErrorCode.WalPending => "WAL_PENDING",
        ErrorCode.Closed => "CLOSED",
        _ => "INTERNAL",
    };

    /// <summary>Copy a NUL-terminated UTF-8 C string into a managed string.</summary>
    internal static string? PtrToString(byte* p)
    {
        if (p == null)
            return null;
        int len = 0;
        while (p[len] != 0)
            len++;
        return Encoding.UTF8.GetString(p, len);
    }

    /// <summary>Copy a library-filled vl_buf into a managed array and free it.</summary>
    internal static byte[] TakeBuf(ref Native.VlBuf buf)
    {
        try
        {
            if (buf.data == null || buf.len == 0)
                return Array.Empty<byte>();
            var managed = new byte[(int)buf.len];
            Marshal.Copy((IntPtr)buf.data, managed, 0, managed.Length);
            return managed;
        }
        finally
        {
            Native.vl_buf_free(ref buf);
        }
    }

    internal static byte[] ToUtf8Nul(string s)
    {
        var bytes = new byte[Encoding.UTF8.GetByteCount(s) + 1];
        Encoding.UTF8.GetBytes(s, 0, s.Length, bytes, 0);
        return bytes; // trailing 0 already present (zero-initialized)
    }

    internal static byte[] JsonBytes<T>(T value) => JsonSerializer.SerializeToUtf8Bytes(value, Json);
}

namespace VecLite;

/// <summary>Stable error codes mirroring the C ABI (SPEC-008 §3, CS-012).</summary>
public enum ErrorCode
{
    Internal = -99,
    Closed = -13,
    WalPending = -12,
    Io = -11,
    InvalidArgument = -10,
    ReadOnly = -9,
    UnsupportedProvider = -8,
    UnsupportedFormat = -7,
    Corrupt = -6,
    Locked = -5,
    DimensionMismatch = -4,
    AlreadyExists = -3,
    VectorNotFound = -2,
    CollectionNotFound = -1,
}

/// <summary>The base exception for every VecLite failure (CS-012).</summary>
public class VecLiteException : Exception
{
    public ErrorCode Code { get; }
    public string CodeString => Interop.CodeString(Code);

    public VecLiteException(ErrorCode code, string message) : base(message)
    {
        Code = code;
    }

    /// <summary>Build the specific subclass for a raw FFI status code.</summary>
    internal static VecLiteException FromCode(int rawCode, string message)
    {
        var code = ToCode(rawCode);
        return code switch
        {
            ErrorCode.DimensionMismatch => new DimensionMismatchException(message),
            ErrorCode.CollectionNotFound => new CollectionNotFoundException(message),
            ErrorCode.AlreadyExists => new AlreadyExistsException(message),
            ErrorCode.Locked => new LockedException(message),
            _ => new VecLiteException(code, message),
        };
    }

    // Unknown/future codes fall back to Internal — forward compatible with a
    // #[non_exhaustive] core (CS-012, acceptance 5).
    private static ErrorCode ToCode(int raw) =>
        Enum.IsDefined(typeof(ErrorCode), raw) ? (ErrorCode)raw : ErrorCode.Internal;
}

/// <summary>A vector's width did not match the collection dimension.</summary>
public sealed class DimensionMismatchException : VecLiteException
{
    public DimensionMismatchException(string message) : base(ErrorCode.DimensionMismatch, message) { }
}

/// <summary>The named collection (or alias) does not exist.</summary>
public sealed class CollectionNotFoundException : VecLiteException
{
    public CollectionNotFoundException(string message) : base(ErrorCode.CollectionNotFound, message) { }
}

/// <summary>A collection or alias with that name already exists.</summary>
public sealed class AlreadyExistsException : VecLiteException
{
    public AlreadyExistsException(string message) : base(ErrorCode.AlreadyExists, message) { }
}

/// <summary>The database file is locked by another process/handle.</summary>
public sealed class LockedException : VecLiteException
{
    public LockedException(string message) : base(ErrorCode.Locked, message) { }
}

using System.Runtime.InteropServices;

namespace VecLite;

/// <summary>
/// A native database handle wrapped in a SafeHandle (CS-010): released exactly
/// once — on Dispose or, as a safety net, under finalization/thread-abort — with
/// no native leak.
/// </summary>
public sealed class VecLiteDbHandle : SafeHandle
{
    public VecLiteDbHandle() : base(IntPtr.Zero, ownsHandle: true) { }

    public override bool IsInvalid => handle == IntPtr.Zero;

    protected override bool ReleaseHandle()
    {
        // Idempotent at the ABI level; SafeHandle guarantees a single call.
        return Native.vl_db_close(handle) == Native.VL_OK;
    }
}

/// <summary>A native collection handle (a lightweight view); freed on Dispose.</summary>
public sealed class VecLiteCollectionHandle : SafeHandle
{
    public VecLiteCollectionHandle() : base(IntPtr.Zero, ownsHandle: true) { }

    public override bool IsInvalid => handle == IntPtr.Zero;

    protected override bool ReleaseHandle()
    {
        return Native.vl_collection_free(handle) == Native.VL_OK;
    }
}

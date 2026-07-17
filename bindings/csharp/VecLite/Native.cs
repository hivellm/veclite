using System.Runtime.InteropServices;

namespace VecLite;

/// <summary>
/// P/Invoke declarations for the VecLite C ABI (SPEC-008). Structured values
/// cross as JSON (VL_CODEC_JSON); vectors as pinned float pointers. A custom
/// import resolver locates the native library under runtimes/&lt;rid&gt;/native/
/// so it loads both from a NuGet package and from a plain project build.
/// </summary>
internal static unsafe partial class Native
{
    internal const string Lib = "veclite_ffi";
    internal const byte CodecJson = 0;

    // Error codes (SPEC-008 §3).
    internal const int VL_OK = 0;

    static Native()
    {
        NativeLibrary.SetDllImportResolver(typeof(Native).Assembly, Resolve);
    }

    /// <summary>Force the static constructor (and thus the resolver) to run.</summary>
    internal static void EnsureInitialized() { }

    private static IntPtr Resolve(string name, System.Reflection.Assembly assembly, DllImportSearchPath? path)
    {
        if (name != Lib)
            return IntPtr.Zero;

        string rid = Rid();
        string file = OperatingSystem.IsWindows() ? $"{Lib}.dll"
            : OperatingSystem.IsMacOS() ? $"lib{Lib}.dylib"
            : $"lib{Lib}.so";

        foreach (var baseDir in new[] { AppContext.BaseDirectory, Path.GetDirectoryName(assembly.Location) ?? "" })
        {
            var candidate = Path.Combine(baseDir, "runtimes", rid, "native", file);
            if (File.Exists(candidate) && NativeLibrary.TryLoad(candidate, out var h))
                return h;
        }
        // Fall back to the default search (app dir / PATH / LD_LIBRARY_PATH).
        return NativeLibrary.TryLoad(name, assembly, path, out var def) ? def : IntPtr.Zero;
    }

    private static string Rid()
    {
        string arch = RuntimeInformation.ProcessArchitecture switch
        {
            Architecture.X64 => "x64",
            Architecture.Arm64 => "arm64",
            _ => "x64",
        };
        string os = OperatingSystem.IsWindows() ? "win"
            : OperatingSystem.IsMacOS() ? "osx"
            : "linux";
        return $"{os}-{arch}";
    }

    // ── result structs (repr(C)) ─────────────────────────────────────────────

    [StructLayout(LayoutKind.Sequential)]
    internal struct VlBuf
    {
        public byte* data;
        public nuint len;
    }

    [StructLayout(LayoutKind.Sequential)]
    internal struct VlHitView
    {
        public byte* id;          // NUL-terminated
        public float score;
        public byte* payload;     // encoded bytes (null when absent)
        public nuint payload_len;
        public byte has_vector;   // C bool as a blittable byte (0/1)
        public float* vector;     // null unless requested
        public nuint vector_len;
    }

    // ── meta / errors ────────────────────────────────────────────────────────
    [LibraryImport(Lib)] internal static partial byte* vl_version();
    [LibraryImport(Lib)] internal static partial uint vl_abi_version();
    [LibraryImport(Lib)] internal static partial uint vl_format_version();
    [LibraryImport(Lib)] internal static partial byte* vl_last_error_message();

    // ── lifecycle ──────────────────────────────────────────────────────────
    [LibraryImport(Lib)] internal static partial int vl_open(byte* path, byte* opts, nuint optsLen, out IntPtr db);
    [LibraryImport(Lib)] internal static partial int vl_open_memory(out IntPtr db);
    [LibraryImport(Lib)] internal static partial int vl_db_close(IntPtr db);
    [LibraryImport(Lib)] internal static partial int vl_db_checkpoint(IntPtr db);
    [LibraryImport(Lib)] internal static partial int vl_db_snapshot(IntPtr db, byte* path);
    [LibraryImport(Lib)] internal static partial int vl_db_vacuum(IntPtr db);
    [LibraryImport(Lib)] internal static partial int vl_db_info(IntPtr db, byte codec, out VlBuf outBuf);

    // ── collections ──────────────────────────────────────────────────────────
    [LibraryImport(Lib)] internal static partial int vl_collection_create(IntPtr db, byte* name, byte* opts, nuint optsLen, byte codec, out IntPtr coll);
    [LibraryImport(Lib)] internal static partial int vl_collection_get(IntPtr db, byte* name, out IntPtr coll);
    [LibraryImport(Lib)] internal static partial int vl_collection_drop(IntPtr db, byte* name);
    [LibraryImport(Lib)] internal static partial int vl_collection_rename(IntPtr db, byte* from, byte* to);
    [LibraryImport(Lib)] internal static partial int vl_collection_free(IntPtr coll);
    [LibraryImport(Lib)] internal static partial int vl_collections_list(IntPtr db, byte codec, out VlBuf outBuf);
    [LibraryImport(Lib)] internal static partial int vl_alias_create(IntPtr db, byte* alias, byte* target);
    [LibraryImport(Lib)] internal static partial int vl_alias_delete(IntPtr db, byte* alias);
    [LibraryImport(Lib)] internal static partial int vl_collection_stats(IntPtr coll, byte codec, out VlBuf outBuf);
    [LibraryImport(Lib)] internal static partial int vl_collection_reindex(IntPtr coll);
    [LibraryImport(Lib)] internal static partial int vl_collection_refit(IntPtr coll);
    [LibraryImport(Lib)] internal static partial int vl_payload_index_create(IntPtr coll, byte* key, byte kind);

    // ── writes ─────────────────────────────────────────────────────────────
    [LibraryImport(Lib)] internal static partial int vl_upsert(IntPtr coll, byte* id, float* vec, nuint dim, byte* payload, nuint payloadLen, byte codec);
    [LibraryImport(Lib)] internal static partial int vl_upsert_batch(IntPtr coll, byte* points, nuint len, byte codec);
    [LibraryImport(Lib)] internal static partial int vl_upsert_text(IntPtr coll, byte* id, byte* text, byte* payload, nuint payloadLen, byte codec);
    [LibraryImport(Lib)] internal static partial int vl_delete(IntPtr coll, byte* id, out byte existed);
    [LibraryImport(Lib)] internal static partial int vl_delete_batch(IntPtr coll, byte* ids, nuint len, byte codec, out ulong deleted);
    [LibraryImport(Lib)] internal static partial int vl_count(IntPtr coll, out ulong outN);

    // ── reads & search ───────────────────────────────────────────────────────
    [LibraryImport(Lib)] internal static partial int vl_get(IntPtr coll, byte* id, byte codec, out VlBuf outBuf);
    [LibraryImport(Lib)] internal static partial int vl_search(IntPtr coll, float* vec, nuint dim, uint limit, byte* opts, nuint optsLen, byte codec, out IntPtr hits);
    [LibraryImport(Lib)] internal static partial int vl_search_text(IntPtr coll, byte* query, uint limit, byte* opts, nuint optsLen, byte codec, out IntPtr hits);
    [LibraryImport(Lib)] internal static partial int vl_hybrid_search(IntPtr coll, byte* opts, nuint optsLen, byte codec, out IntPtr hits);
    [LibraryImport(Lib)] internal static partial int vl_scroll(IntPtr coll, byte* opts, nuint len, byte codec, out IntPtr page);

    // ── results ──────────────────────────────────────────────────────────────
    [LibraryImport(Lib)] internal static partial uint vl_hits_len(IntPtr hits);
    [LibraryImport(Lib)] internal static partial int vl_hits_get(IntPtr hits, uint i, out VlHitView view);
    [LibraryImport(Lib)] internal static partial void vl_hits_free(IntPtr hits);
    [LibraryImport(Lib)] internal static partial uint vl_page_len(IntPtr page);
    [LibraryImport(Lib)] internal static partial int vl_page_point(IntPtr page, uint i, out VlBuf outBuf);
    [LibraryImport(Lib)] internal static partial byte* vl_page_cursor(IntPtr page);
    [LibraryImport(Lib)] internal static partial void vl_page_free(IntPtr page);
    [LibraryImport(Lib)] internal static partial void vl_buf_free(ref VlBuf buf);

    // ── chunker ──────────────────────────────────────────────────────────────
    [LibraryImport(Lib)] internal static partial int vl_chunk(byte* text, byte* opts, nuint optsLen, byte codec, out VlBuf outBuf);
}

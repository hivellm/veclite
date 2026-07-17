using System.Runtime.InteropServices;
using System.Text.Json;
using System.Text.Json.Nodes;

namespace VecLite;

/// <summary>
/// A VecLite database (SPEC-011). Wraps the native handle in a SafeHandle;
/// Dispose is idempotent and releases the file lock. Safe for concurrent use by
/// multiple threads (FFI-001).
/// </summary>
public sealed unsafe class Database : IDisposable
{
    private readonly VecLiteDbHandle _handle;

    private Database(VecLiteDbHandle handle) => _handle = handle;

    /// <summary>Core semver string.</summary>
    public static string Version()
    {
        Native.EnsureInitialized();
        return Interop.PtrToString(Native.vl_version()) ?? "";
    }

    /// <summary>C ABI version (loaders gate on it).</summary>
    public static uint AbiVersion() { Native.EnsureInitialized(); return Native.vl_abi_version(); }

    /// <summary>On-disk storage format version.</summary>
    public static uint FormatVersion() { Native.EnsureInitialized(); return Native.vl_format_version(); }

    /// <summary>Open (or create) a durable single-file database.</summary>
    public static Database Open(string path, OpenOptions? _ = null)
    {
        Native.EnsureInitialized();
        var pathBytes = Interop.ToUtf8Nul(path);
        IntPtr raw;
        fixed (byte* p = pathBytes)
        {
            // Tuned options are not yet plumbed through the C ABI; open defaults.
            Interop.Check(Native.vl_open(p, null, 0, out raw));
        }
        return Wrap(raw);
    }

    /// <summary>Open an ephemeral in-memory database.</summary>
    public static Database Memory()
    {
        Native.EnsureInitialized();
        Interop.Check(Native.vl_open_memory(out IntPtr raw));
        return Wrap(raw);
    }

    private static Database Wrap(IntPtr raw)
    {
        var handle = new VecLiteDbHandle();
        Marshal.InitHandle(handle, raw);
        return new Database(handle);
    }

    internal IntPtr Raw => _handle.DangerousGetHandle();

    public void Dispose() => _handle.Dispose();

    /// <summary>Force a WAL checkpoint.</summary>
    public void Checkpoint()
    {
        Interop.Check(Native.vl_db_checkpoint(Raw));
        GC.KeepAlive(this);
    }

    /// <summary>Write a consistent copy of the whole database to path.</summary>
    public void Snapshot(string path)
    {
        var pb = Interop.ToUtf8Nul(path);
        fixed (byte* p = pb)
            Interop.Check(Native.vl_db_snapshot(Raw, p));
        GC.KeepAlive(this);
    }

    /// <summary>Reclaim space from tombstoned/rewritten data.</summary>
    public void Vacuum()
    {
        Interop.Check(Native.vl_db_vacuum(Raw));
        GC.KeepAlive(this);
    }

    /// <summary>Create a collection and return a handle to it.</summary>
    public Collection CreateCollection(string name, CollectionOptions options)
    {
        var opts = new Dictionary<string, object?>
        {
            ["dimension"] = options.Dimension,
            ["metric"] = options.Metric.Wire(),
        };
        if (options.QuantizationBits is byte q) opts["quantization_bits"] = q;
        if (options.EmbeddingProvider is string prov) opts["embedding_provider"] = prov;

        var nameBytes = Interop.ToUtf8Nul(name);
        var optBytes = Interop.JsonBytes(opts);
        IntPtr raw;
        fixed (byte* n = nameBytes)
        fixed (byte* o = optBytes)
            Interop.Check(Native.vl_collection_create(Raw, n, o, (nuint)optBytes.Length, Native.CodecJson, out raw));
        GC.KeepAlive(this);
        return Collection.Wrap(this, raw);
    }

    /// <summary>Get a handle to an existing collection (or alias).</summary>
    public Collection GetCollection(string name)
    {
        var nameBytes = Interop.ToUtf8Nul(name);
        IntPtr raw;
        fixed (byte* n = nameBytes)
            Interop.Check(Native.vl_collection_get(Raw, n, out raw));
        GC.KeepAlive(this);
        return Collection.Wrap(this, raw);
    }

    /// <summary>Drop a collection.</summary>
    public void DropCollection(string name)
    {
        var nb = Interop.ToUtf8Nul(name);
        fixed (byte* n = nb)
            Interop.Check(Native.vl_collection_drop(Raw, n));
        GC.KeepAlive(this);
    }

    /// <summary>Rename a collection.</summary>
    public void RenameCollection(string from, string to)
    {
        var fb = Interop.ToUtf8Nul(from);
        var tb = Interop.ToUtf8Nul(to);
        fixed (byte* f = fb)
        fixed (byte* t = tb)
            Interop.Check(Native.vl_collection_rename(Raw, f, t));
        GC.KeepAlive(this);
    }

    /// <summary>Create an alias resolving to target.</summary>
    public void CreateAlias(string alias, string target)
    {
        var ab = Interop.ToUtf8Nul(alias);
        var tb = Interop.ToUtf8Nul(target);
        fixed (byte* a = ab)
        fixed (byte* t = tb)
            Interop.Check(Native.vl_alias_create(Raw, a, t));
        GC.KeepAlive(this);
    }

    /// <summary>Remove an alias.</summary>
    public void DeleteAlias(string alias)
    {
        var ab = Interop.ToUtf8Nul(alias);
        fixed (byte* a = ab)
            Interop.Check(Native.vl_alias_delete(Raw, a));
        GC.KeepAlive(this);
    }

    /// <summary>Sorted collection names.</summary>
    public IReadOnlyList<string> ListCollections()
    {
        Interop.Check(Native.vl_collections_list(Raw, Native.CodecJson, out var buf));
        GC.KeepAlive(this);
        var raw = Interop.TakeBuf(ref buf);
        return JsonSerializer.Deserialize<List<string>>(raw, Interop.Json) ?? new List<string>();
    }

    /// <summary>A database overview (format version, collections, aliases).</summary>
    public JsonNode Info()
    {
        Interop.Check(Native.vl_db_info(Raw, Native.CodecJson, out var buf));
        GC.KeepAlive(this);
        var raw = Interop.TakeBuf(ref buf);
        return JsonNode.Parse(raw) ?? new JsonObject();
    }
}

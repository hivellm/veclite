// C# conformance runner over the shared YAML corpus (SPEC-015 §3). Mirrors the
// Rust/Python/Node/Go runners: same ops, same canonical observation shapes, same
// golden (numeric tolerance 1e-5). Drives the VecLite .NET binding as a user
// would.
//
//   dotnet run [corpus_dir]
using System.Text.Json;
using System.Text.Json.Nodes;
using VecLite;
using YamlDotNet.Serialization;

const double Tol = 1e-5;

string corpus = args.Length > 0 ? args[0] : "../../corpus";
var golden = JsonNode.Parse(File.ReadAllText(Path.Combine(corpus, "golden.json")))!.AsObject();
var deserializer = new DeserializerBuilder().Build();

var files = Directory.GetFiles(corpus, "*.yaml").OrderBy(f => f).ToArray();
if (files.Length == 0)
{
    Console.Error.WriteLine($"[conformance:csharp] no *.yaml under {corpus}");
    return 1;
}

int total = 0, failed = 0;
foreach (var file in files)
{
    var suite = deserializer.Deserialize<Dictionary<object, object>>(File.ReadAllText(file));
    var cases = (List<object>)suite["cases"];
    foreach (var caseObj in cases)
    {
        total++;
        var c = (Dictionary<object, object>)caseObj;
        string id = (string)c["id"];
        var g = golden.TryGetPropertyValue(id, out var gv) ? gv as JsonArray : null;
        var errs = RunCase(c, g);
        if (errs.Count > 0)
        {
            failed++;
            foreach (var e in errs)
                Console.Error.WriteLine($"[conformance:csharp] FAIL {e}");
        }
    }
}

if (failed > 0)
{
    Console.Error.WriteLine($"[conformance:csharp] {failed}/{total} cases FAILED");
    return 1;
}
Console.Error.WriteLine($"[conformance:csharp] PASS — {total} cases across {files.Length} files");
return 0;

List<string> RunCase(Dictionary<object, object> c, JsonArray? goldenArr)
{
    string mode = c.TryGetValue("mode", out var m) ? (string)m : "both";
    bool[] modes = mode switch { "memory" => new[] { false }, "file" => new[] { true }, _ => new[] { false, true } };
    var errs = new List<string>();
    foreach (var fileMode in modes)
        errs.AddRange(RunCaseInMode(c, fileMode, goldenArr));
    return errs;
}

List<string> RunCaseInMode(Dictionary<object, object> c, bool fileMode, JsonArray? golden)
{
    var errs = new List<string>();
    string id = (string)c["id"];
    string? dir = null, path = null;
    if (fileMode)
    {
        dir = Directory.CreateTempSubdirectory("veclite-conf-cs-").FullName;
        path = Path.Combine(dir, "db.veclite");
    }

    Database db;
    try { db = OpenDb(path); }
    catch (VecLiteException e) { if (dir != null) Directory.Delete(dir, true); return new() { $"{id}: open ({(fileMode ? "file" : "memory")}): {e.CodeString}" }; }

    try
    {
        int idx = 0;
        var steps = (List<object>)c["steps"];
        for (int i = 0; i < steps.Count; i++)
        {
            var step = (Dictionary<object, object>)steps[i];
            string op = (string)step["op"];
            string where = $"{id}: step {i} `{op}`";

            if (op == "reopen")
            {
                if (path == null) { errs.Add(where + ": reopen requires file mode"); break; }
                db.Dispose();
                db = OpenDb(path);
                continue;
            }

            var stepArgs = step.TryGetValue("args", out var av) ? Coerce.Dict(av) : new();
            var obs = Ops.Execute(db, op, stepArgs);

            if (step.TryGetValue("expect", out var expectObj))
            {
                foreach (var (key, want) in Coerce.Dict(expectObj))
                {
                    if (key == "error")
                    {
                        var got = obs["error"]?.GetValue<string>();
                        if (got != Coerce.Str(want)) errs.Add($"{where}: expected error {want}, got {obs.ToJsonString()}");
                    }
                    else if (!MatchesSubset(Coerce.ToNode(want), obs[key]))
                    {
                        errs.Add($"{where}: `{key}`: expected {Coerce.ToNode(want)?.ToJsonString()}, got {obs[key]?.ToJsonString()}");
                    }
                }
            }
            else if (obs["error"] != null)
            {
                errs.Add($"{where}: unexpected error {obs["error"]}");
            }

            if (golden != null)
            {
                if (idx >= golden.Count) errs.Add(where + ": golden has no entry (re-bless)");
                else if (!EqTol(golden[idx], obs)) errs.Add($"{where}: golden mismatch: {golden[idx]?.ToJsonString()} != {obs.ToJsonString()}");
            }
            idx++;
        }
    }
    finally
    {
        db.Dispose();
        if (dir != null) Directory.Delete(dir, true);
    }
    return errs;

    static Database OpenDb(string? p) => p == null ? Database.Memory() : Database.Open(p);
}

static bool EqTol(JsonNode? want, JsonNode? got)
{
    if (want is null || got is null) return want is null && got is null;
    if (want is JsonObject wo)
    {
        if (got is not JsonObject go || go.Count != wo.Count) return false;
        foreach (var (k, wv) in wo)
            if (!go.TryGetPropertyValue(k, out var gv) || !EqTol(wv, gv)) return false;
        return true;
    }
    if (want is JsonArray wa)
    {
        if (got is not JsonArray ga || ga.Count != wa.Count) return false;
        for (int i = 0; i < wa.Count; i++)
            if (!EqTol(wa[i], ga[i])) return false;
        return true;
    }
    return ScalarEq(want!, got!);
}

static bool MatchesSubset(JsonNode? want, JsonNode? got)
{
    if (want is null) return got is null;
    if (want is JsonObject wo)
    {
        if (got is not JsonObject go) return false;
        foreach (var (k, wv) in wo)
        {
            go.TryGetPropertyValue(k, out var gv);
            if (!MatchesSubset(wv, gv)) return false;
        }
        return true;
    }
    if (want is JsonArray wa)
    {
        if (got is not JsonArray ga || ga.Count != wa.Count) return false;
        for (int i = 0; i < wa.Count; i++)
            if (!MatchesSubset(wa[i], ga[i])) return false;
        return true;
    }
    return ScalarEq(want, got!);
}

static bool ScalarEq(JsonNode want, JsonNode? got)
{
    if (got is null) return false;
    var wv = want.AsValue();
    var gv = got.AsValue();
    if (wv.TryGetValue<double>(out var wd) && gv.TryGetValue<double>(out var gd) && IsNumeric(wv) && IsNumeric(gv))
        return Math.Abs(wd - gd) <= Tol;
    if (wv.TryGetValue<bool>(out var wb) && gv.TryGetValue<bool>(out var gb)) return wb == gb;
    return wv.ToJsonString() == gv.ToJsonString();
}

static bool IsNumeric(JsonValue v) =>
    v.TryGetValue<double>(out _) && !v.TryGetValue<bool>(out _) && !v.TryGetValue<string>(out _);

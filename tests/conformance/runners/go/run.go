// Go conformance runner over the shared YAML corpus (SPEC-015 §3). Mirrors the
// Python/Node runners: same ops, same canonical observation shapes, same golden
// (numeric tolerance 1e-5). Drives the veclite-go binding as a user would.
//
//	go run . [corpus_dir]
package main

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"sort"

	"github.com/hivellm/veclite-go"
	"gopkg.in/yaml.v3"
)

const tol = 1e-5

type suite struct {
	Cases []caseDef `yaml:"cases"`
}

type caseDef struct {
	ID    string `yaml:"id"`
	Mode  string `yaml:"mode"`
	Steps []step `yaml:"steps"`
}

type step struct {
	Op     string         `yaml:"op"`
	Args   map[string]any `yaml:"args"`
	Expect map[string]any `yaml:"expect"`
}

func main() {
	corpus := "../../corpus"
	if len(os.Args) > 1 {
		corpus = os.Args[1]
	}
	goldenRaw, err := os.ReadFile(filepath.Join(corpus, "golden.json"))
	must(err)
	var golden map[string][]any
	must(json.Unmarshal(goldenRaw, &golden))

	files, err := filepath.Glob(filepath.Join(corpus, "*.yaml"))
	must(err)
	sort.Strings(files)
	if len(files) == 0 {
		fmt.Fprintf(os.Stderr, "[conformance:go] no *.yaml under %s\n", corpus)
		os.Exit(1)
	}

	total, failed := 0, 0
	for _, f := range files {
		data, err := os.ReadFile(f)
		must(err)
		var s suite
		must(yaml.Unmarshal(data, &s))
		for _, c := range s.Cases {
			total++
			errs := runCase(c, golden[c.ID])
			if len(errs) > 0 {
				failed++
				for _, e := range errs {
					fmt.Fprintf(os.Stderr, "[conformance:go] FAIL %s\n", e)
				}
			}
		}
	}
	if failed > 0 {
		fmt.Fprintf(os.Stderr, "[conformance:go] %d/%d cases FAILED\n", failed, total)
		os.Exit(1)
	}
	fmt.Fprintf(os.Stderr, "[conformance:go] PASS — %d cases across %d files\n", total, len(files))
}

func runCase(c caseDef, golden []any) []string {
	mode := c.Mode
	if mode == "" {
		mode = "both"
	}
	var modes []bool // false = memory, true = file
	switch mode {
	case "both":
		modes = []bool{false, true}
	case "memory":
		modes = []bool{false}
	case "file":
		modes = []bool{true}
	}
	var errs []string
	for _, fileMode := range modes {
		errs = append(errs, runCaseInMode(c, fileMode, golden)...)
	}
	return errs
}

func runCaseInMode(c caseDef, fileMode bool, golden []any) []string {
	var errs []string
	var dir, path string
	if fileMode {
		var err error
		dir, err = os.MkdirTemp("", "veclite-conf-go-")
		if err != nil {
			return []string{c.ID + ": mkdtemp: " + err.Error()}
		}
		defer os.RemoveAll(dir)
		path = filepath.Join(dir, "db.veclite")
	}

	db, err := openDB(path)
	if err != nil {
		return []string{fmt.Sprintf("%s: open (%s): %v", c.ID, modeName(fileMode), err)}
	}
	defer func() { _ = db.Close() }()

	idx := 0
	for i, st := range c.Steps {
		where := fmt.Sprintf("%s: step %d `%s`", c.ID, i, st.Op)
		if st.Op == "reopen" {
			if !fileMode {
				errs = append(errs, where+": reopen requires file mode")
				break
			}
			if err := db.Close(); err != nil {
				errs = append(errs, where+": close: "+err.Error())
			}
			db, err = openDB(path)
			if err != nil {
				errs = append(errs, where+": reopen: "+err.Error())
				break
			}
			continue
		}

		obs := execute(db, st.Op, st.Args)

		if st.Expect != nil {
			for key, want := range st.Expect {
				if key == "error" {
					if obs["error"] != want {
						errs = append(errs, fmt.Sprintf("%s: expected error %v, got %v", where, want, obs))
					}
				} else if !matchesSubset(want, obs[key]) {
					errs = append(errs, fmt.Sprintf("%s: `%s`: expected %v, got %v", where, key, want, obs[key]))
				}
			}
		} else if _, isErr := obs["error"]; isErr {
			errs = append(errs, fmt.Sprintf("%s: unexpected error %v", where, obs["error"]))
		}

		if golden != nil {
			if idx >= len(golden) {
				errs = append(errs, where+": golden has no entry (re-bless)")
			} else if !eqTol(golden[idx], canonical(obs)) {
				errs = append(errs, fmt.Sprintf("%s: golden mismatch: %v != %v", where, golden[idx], obs))
			}
		}
		idx++
	}
	return errs
}

func modeName(fileMode bool) string {
	if fileMode {
		return "file"
	}
	return "memory"
}

func openDB(path string) (*veclite.Database, error) {
	if path == "" {
		return veclite.Memory(), nil
	}
	return veclite.Open(path, nil)
}

func must(err error) {
	if err != nil {
		panic(err)
	}
}

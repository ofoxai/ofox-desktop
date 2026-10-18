// Purely synthetic cross-implementation fixtures. Loaded with go -overlay;
// this file does not modify the MIT-licensed Magpie checkout.
package sessions

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"math"
	"os"
	"path/filepath"
	"reflect"
	"sort"
	"strconv"
	"strings"
	"testing"
	"time"

	"github.com/yetone/magpie/internal/agentenv"
	"github.com/yetone/magpie/internal/catalog"
	"github.com/yetone/magpie/internal/settings"
	"github.com/yetone/magpie/internal/testenv"
)

const parityReferenceCommit = "bfaedf18301d02a423bd5422d22e41432d231ab8"

type parityTokens struct {
	Input      int `json:"input"`
	Output     int `json:"output"`
	CacheRead  int `json:"cacheRead"`
	CacheWrite int `json:"cacheWrite"`
	Main       int `json:"main"`
}

func normalizedTokens(v Tokens) parityTokens {
	return parityTokens{v.Input, v.Output, v.CacheRead, v.CacheWrite, v.Input + v.Output}
}

type parityDay struct {
	Date                   string           `json:"date"`
	Tokens                 parityTokens     `json:"tokens"`
	ActiveMs               int64            `json:"activeMs"`
	ReferenceActiveSeconds int64            `json:"referenceActiveSeconds"`
	HourActiveMs           map[string]int64 `json:"hourActiveMs"`
	CostUsd                string           `json:"costUsd"`
}

type parityThread struct {
	ID                     string       `json:"id"`
	Tokens                 parityTokens `json:"tokens"`
	ActiveMs               int64        `json:"activeMs"`
	ReferenceActiveSeconds int64        `json:"referenceActiveSeconds"`
	CostUsd                string       `json:"costUsd"`
}

type parityReport struct {
	Version                int               `json:"version"`
	ReferenceCommit        string            `json:"referenceCommit"`
	Timezone               string            `json:"timezone"`
	ThreadCount            int               `json:"threadCount"`
	Tokens                 parityTokens      `json:"tokens"`
	Quantiles              map[string]int    `json:"quantiles"`
	CostUsd                string            `json:"costUsd"`
	ActiveMs               int64             `json:"activeMs"`
	ReferenceActiveSeconds int64             `json:"referenceActiveSeconds"`
	Days                   []parityDay       `json:"days"`
	Threads                []parityThread    `json:"threads"`
	ActivityCases          []map[string]any  `json:"activityCases,omitempty"`
	CallCount              int               `json:"callCount,omitempty"`
	CallTokens             parityTokens      `json:"callTokens"`
	SourceSha256           map[string]string `json:"sourceSha256,omitempty"`
	ReferenceLimitations   map[string]any    `json:"referenceLimitations,omitempty"`
}

type parityManifest struct {
	Version         int    `json:"version"`
	ReferenceCommit string `json:"referenceCommit"`
	Synthetic       bool   `json:"synthetic"`
	Timezone        string `json:"timezone"`
	Now             string `json:"now"`
	Files           []struct {
		Path string `json:"path"`
	} `json:"files"`
	Prices map[string]struct {
		Input, Output, CacheRead, CacheWrite float64
	} `json:"pricesUsdPerMillion"`
	ActivityCases []struct {
		ID               string   `json:"id"`
		Timezone         string   `json:"timezone"`
		Main             bool     `json:"main"`
		Timestamps       []string `json:"timestamps"`
		ExpectedActiveMs int64    `json:"expectedActiveMs"`
	} `json:"activityCases"`
}

func parityCost(value float64) string {
	return strings.TrimRight(strings.TrimRight(strconv.FormatFloat(value, 'f', 12, 64), "0"), ".")
}

func readParityJSON(t *testing.T, path string, value any) {
	t.Helper()
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if err := json.Unmarshal(data, value); err != nil {
		t.Fatal(err)
	}
}

func TestOfoxFrozenUsageParity(t *testing.T) {
	testenv.Offline()
	dir := os.Getenv("OFOX_PARITY_FIXTURES")
	output := os.Getenv("OFOX_PARITY_REFERENCE_OUTPUT")
	if dir == "" || output == "" {
		t.Fatal("explicit synthetic fixture and output paths required")
	}
	var manifest parityManifest
	readParityJSON(t, filepath.Join(dir, "manifest.json"), &manifest)
	if manifest.Version != 1 || !manifest.Synthetic || manifest.ReferenceCommit != parityReferenceCommit {
		t.Fatal("unexpected fixture provenance")
	}
	loc, err := time.LoadLocation(manifest.Timezone)
	if err != nil {
		t.Fatal(err)
	}
	testenv.Zone(t, loc)
	home := t.TempDir()
	for _, name := range agentenv.Vars {
		t.Setenv(name, "")
	}
	for _, name := range []string{"HOME", "USERPROFILE", "XDG_CONFIG_HOME", "XDG_CACHE_HOME", "XDG_DATA_HOME", "XDG_STATE_HOME", "APPDATA", "LOCALAPPDATA"} {
		path := filepath.Join(home, name)
		if err := os.MkdirAll(path, 0700); err != nil {
			t.Fatal(err)
		}
		t.Setenv(name, path)
	}
	codex := filepath.Join(home, "codex")
	t.Setenv("CODEX_HOME", codex)
	report := parityReport{Version: 1, ReferenceCommit: parityReferenceCommit, Timezone: manifest.Timezone, SourceSha256: map[string]string{}, Days: []parityDay{}, Threads: []parityThread{}}
	for _, source := range manifest.Files {
		if filepath.Base(source.Path) != source.Path {
			t.Fatal("fixture filenames must be relative basenames")
		}
		data, err := os.ReadFile(filepath.Join(dir, source.Path))
		if err != nil {
			t.Fatal(err)
		}
		hash := sha256.Sum256(data)
		report.SourceSha256[source.Path] = hex.EncodeToString(hash[:])
		path := filepath.Join(codex, "sessions", source.Path)
		if err := os.MkdirAll(filepath.Dir(path), 0700); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(path, data, 0600); err != nil {
			t.Fatal(err)
		}
	}
	previousPrice := PriceOf
	PriceOf = func(_ settings.Settings, model string) (catalog.Price, bool) {
		p, ok := manifest.Prices[model]
		return catalog.Price{Input: p.Input, Output: p.Output, CacheRead: p.CacheRead, CacheWrite: p.CacheWrite}, ok
	}
	Reset()
	t.Cleanup(func() { PriceOf = previousPrice; Reset() })
	now, err := time.Parse(time.RFC3339Nano, manifest.Now)
	if err != nil {
		t.Fatal(err)
	}
	stats := StatsAt(0, now)
	overview := stats.Overview("codex", "", "", 100)
	rollup := stats.Sum("", "")
	report.ThreadCount, report.Tokens = overview.Count, normalizedTokens(rollup.Tokens)
	report.Quantiles = map[string]int{"median": overview.Median, "p90": overview.P90}
	report.CostUsd, report.ReferenceActiveSeconds = parityCost(rollup.Cost), rollup.Active
	rawDays := map[string]*parityDay{}
	rawThreads := map[string]int64{}
	for _, source := range codexFilesIn(codex) {
		state := parse(source, nil)
		for date, day := range state.Days {
			if day.Active <= 0 {
				continue
			}
			if rawDays[date] == nil {
				rawDays[date] = &parityDay{Date: date, HourActiveMs: map[string]int64{}}
			}
			rawDays[date].ActiveMs += day.Active
			rawThreads[strings.TrimPrefix(source.key, "codex:")] += day.Active
			report.ActiveMs += day.Active
			for hour, value := range day.Hours {
				if value != 0 {
					rawDays[date].HourActiveMs[strconv.Itoa(hour)] += value
				}
			}
		}
	}
	for _, day := range rollup.Days {
		row := rawDays[day.Date]
		if row == nil {
			row = &parityDay{Date: day.Date, HourActiveMs: map[string]int64{}}
		}
		row.Tokens, row.CostUsd, row.ReferenceActiveSeconds = normalizedTokens(day.Tokens), parityCost(day.Cost), day.Active
		report.Days = append(report.Days, *row)
	}
	for _, thread := range stats.Sessions {
		if thread.Agent != "codex" {
			t.Fatal("unexpected non-synthetic client source")
		}
		report.Threads = append(report.Threads, parityThread{ID: thread.ID, Tokens: normalizedTokens(thread.Tokens), ActiveMs: rawThreads[thread.ID], ReferenceActiveSeconds: thread.Active, CostUsd: parityCost(thread.Cost)})
	}
	sort.Slice(report.Threads, func(i, j int) bool { return report.Threads[i].ID < report.Threads[j].ID })
	var callTokens Tokens
	for _, call := range Calls(time.Time{}) {
		report.CallCount++
		callTokens.add(call.Tokens)
	}
	report.CallTokens = normalizedTokens(callTokens)
	if report.CallTokens != report.Tokens {
		t.Errorf("reference call and summary tokens disagree: calls=%+v summary=%+v", report.CallTokens, report.Tokens)
	}
	for _, tc := range manifest.ActivityCases {
		t.Run(tc.ID, func(t *testing.T) {
			zone, err := time.LoadLocation(tc.Timezone)
			if err != nil {
				t.Fatal(err)
			}
			testenv.Zone(t, zone)
			state := &state{}
			for _, at := range tc.Timestamps {
				stamp, err := time.Parse(time.RFC3339Nano, at)
				if err != nil {
					t.Fatal(err)
				}
				state.saw(stamp, tc.Main)
			}
			var active int64
			byDate := map[string]int64{}
			hours := map[string]map[string]int64{}
			for date, day := range state.Days {
				active += day.Active
				byDate[date] = day.Active
				hours[date] = map[string]int64{}
				for hour, ms := range day.Hours {
					if ms != 0 {
						hours[date][strconv.Itoa(hour)] = ms
					}
				}
			}
			if active != tc.ExpectedActiveMs {
				t.Errorf("activity=%d want=%d", active, tc.ExpectedActiveMs)
			}
			report.ActivityCases = append(report.ActivityCases, map[string]any{"id": tc.ID, "timezone": tc.Timezone, "activeMs": active, "dayActiveMs": byDate, "hourActiveMs": hours})
		})
	}
	var alias cxUsage
	if err := json.Unmarshal([]byte(`{"input_tokens":100,"output_tokens":10,"cache_read_input_tokens":40,"cache_write_input_tokens":0,"reasoning_output_tokens":0}`), &alias); err != nil {
		t.Fatal(err)
	}
	if alias.Cached != 0 || alias.Known&4 != 0 {
		t.Fatal("reference support for the alternate cache counter changed; reclassify this comparison")
	}
	var missing cxUsage
	if err := json.Unmarshal([]byte(`{"input_tokens":100,"output_tokens":10,"cached_input_tokens":40,"reasoning_output_tokens":0}`), &missing); err != nil {
		t.Fatal(err)
	}
	report.ReferenceLimitations = map[string]any{
		"alternateCacheReadCounter": map[string]any{"field": "cache_read_input_tokens", "recognized": false, "referenceTokens": normalizedTokens(spent(alias.raw())), "reason": "the pinned reference only decodes cached_input_tokens; do not treat its absent cache as observed zero"},
		"missingCacheWriteCounter":  map[string]any{"counterKnownMask": missing.Known, "referenceCacheWrite": missing.CacheWrite, "reason": "the pinned summary retains zero in the numeric bucket; Ofox must separately report incomplete coverage"},
	}
	data, err := json.MarshalIndent(report, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(output, append(data, '\n'), 0600); err != nil {
		t.Fatal(err)
	}
	var expected parityReport
	readParityJSON(t, filepath.Join(dir, "expected.json"), &expected)
	actualComparable := report
	actualComparable.CallCount, actualComparable.CallTokens, actualComparable.SourceSha256, actualComparable.ActivityCases, actualComparable.ReferenceLimitations = 0, parityTokens{}, nil, nil, nil
	if !reflect.DeepEqual(actualComparable, expected) {
		t.Errorf("reference differs from independently calculated synthetic expectations; inspect %s", output)
	}
	if math.IsNaN(rollup.Cost) {
		t.Fatal("non-finite reference cost")
	}
	fmt.Printf("OFOX_PARITY_REFERENCE: %d threads, %d main Tokens, %d ms active, $%s\n", report.ThreadCount, report.Tokens.Main, report.ActiveMs, report.CostUsd)
}

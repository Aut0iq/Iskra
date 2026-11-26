-- Tests for plugin/iskra/model.luau, run under plain Lua 5.4+:
--   lua plugin/iskra/tests/model_test.lua        (from the repo root)
--   lua tests/model_test.lua                     (from plugin/iskra)
-- model.luau is intentionally Lua 5.4-clean so this runs without Luau.

local here = (arg and arg[0] or ""):match("^(.*)[/\\][^/\\]*$") or "."
local model = dofile(here .. "/../model.luau")

local checks, failures = 0, 0
local function fail(name, got, want)
	failures = failures + 1
	print(string.format("FAIL %s: got %s, want %s", name, tostring(got), tostring(want)))
end
local function eq(got, want, name)
	checks = checks + 1
	if got ~= want then fail(name, got, want) end
end
local function ok(cond, name)
	checks = checks + 1
	if not cond then fail(name, cond, true) end
end

-- ── status() ─────────────────────────────────────────────────────────
eq(model.status(nil), "unknown", "status: nil")
eq(model.status("working"), "unknown", "status: non-table")
eq(model.status({}), "unknown", "status: missing")
eq(model.status({ status = "working" }), "working", "status: working")
eq(model.status({ status = "awaitingInput" }), "awaitingInput", "status: awaitingInput")
eq(model.status({ status = "errored" }), "errored", "status: errored")
eq(model.status({ status = "completed" }), "completed", "status: completed")
eq(model.status({ status = "idle" }), "idle", "status: idle")
eq(model.status({ status = "bogus" }), "unknown", "status: unrecognised")
eq(model.status({ status = 42 }), "unknown", "status: non-string")
eq(model.status({ status = "working", statusStale = true }), "unknown", "status: statusStale")
eq(model.status({ status = "working", status_stale = true }), "unknown", "status: status_stale")
eq(model.status({ status = "working", statusStale = false }), "working", "status: statusStale false")
-- history is ignored: a finished turn under an idle chat stays idle
eq(model.status({ status = "idle", lastTurnOutcome = "completed" }), "idle", "status: lastTurnOutcome ignored")
eq(model.status({ status = "completed", lastTurnOutcome = "errored" }), "completed", "status: lastTurnOutcome never downgrades")

-- ── aggregate() ──────────────────────────────────────────────────────
eq(model.aggregate({}), "idle", "aggregate: empty map idle")
eq(model.aggregate(nil), "idle", "aggregate: nil idle")
eq(model.aggregate({ a = { status = "idle" } }), "idle", "aggregate: idle")
eq(model.aggregate({ a = { status = "working" }, b = { status = "idle" } }), "working", "aggregate: working over idle")
eq(model.aggregate({ a = { status = "working" }, b = { status = "errored" } }), "errored", "aggregate: errored over working")
eq(model.aggregate({ a = { status = "awaitingInput" }, b = { status = "errored" }, c = { status = "working" } }),
	"awaitingInput", "aggregate: awaitingInput first")
eq(model.aggregate({ a = { status = "completed" }, b = {} }), "completed", "aggregate: completed over unknown")
eq(model.aggregate({ a = {} }), "unknown", "aggregate: unknown")
eq(model.aggregate({ a = "junk" }), "unknown", "aggregate: non-table unknown")
-- a stale-marked chat counts as unknown: loses to completed, beats idle
eq(model.aggregate({ a = { status = "completed", statusStale = true }, b = { status = "idle" } }),
	"unknown", "aggregate: stale counts unknown")
eq(model.aggregate({ a = { status = "completed", statusStale = true }, b = { status = "completed" } }),
	"completed", "aggregate: completed outranks stale-unknown")
eq(model.aggregate({ a = { status = "idle", lastTurnOutcome = "completed" } }), "idle", "aggregate: history ignored")

-- ── anim_key() ───────────────────────────────────────────────────────
eq(model.anim_key("working"), "standing_focus_laptop", "anim: working")
eq(model.anim_key("working", true), "standing_focus_solder", "anim: working alternate")
eq(model.anim_key("awaitingInput"), "standing_question", "anim: awaitingInput")
eq(model.anim_key("errored"), "standing_sadness", "anim: errored")
eq(model.anim_key("completed"), "standing_joy", "anim: completed")
eq(model.anim_key("idle"), "standing_idle", "anim: idle")
-- v6 has no standing_subdued: unknown draws standing_idle, never a
-- missing asset; the caption carries "unknown"
eq(model.anim_key("unknown"), "standing_idle", "anim: unknown is standing_idle")
ok(not string.find(model.anim_key("unknown"), "subdued"), "anim: no subdued key")
eq(model.anim_key("whatever"), "standing_idle", "anim: unrecognised")

-- ── usage_rows() ─────────────────────────────────────────────────────
local NOW = 1700000000000 -- epoch ms
local function rec(r, fresh)
	r.active = r.active == nil and true or r.active
	if r.fetched_at == nil and fresh ~= false then
		r.fetched_at = NOW
	end
	return r
end

-- active-only and ordering: provider alphabetic, account as tie-break
local rows = model.usage_rows({
	rec { provider = "zeta", account = "a", windows = {} },
	rec { provider = "alpha", account = "b", windows = {} },
	rec { provider = "alpha", account = "a", windows = {} },
	{ provider = "inactive", active = false, windows = {} },
	{ provider = "unset", windows = {} }, -- no active field: dropped
}, NOW)
eq(#rows, 3, "usage: active records only")
eq(rows[1].provider .. "/" .. rows[1].account, "alpha/a", "usage: sort provider then account")
eq(rows[2].provider .. "/" .. rows[2].account, "alpha/b", "usage: sort 2")
eq(rows[3].provider, "zeta", "usage: sort 3")

-- empty input: no rows, and certainly not a fake 0%
eq(#model.usage_rows({}, NOW), 0, "usage: empty records no rows")
eq(#model.usage_rows(nil, NOW), 0, "usage: nil no rows")
eq(#model.usage_rows("junk", NOW), 0, "usage: non-table no rows")

-- numbers already percent: shown rounded, never multiplied
local r = model.usage_rows({ rec { provider = "p", windows = {
	{ label = "a", used = 79.4 }, { label = "b", used = 0 }, { label = "c", used = 100 },
	{ label = "d", used = 80 }, { label = "e", used = 0.79 },
} } }, NOW)[1]
eq(r.windows[1].display, "79%", "usage: 79.4 rounds to 79%")
eq(r.windows[2].display, "0%", "usage: 0 is valid")
eq(r.windows[3].display, "100%", "usage: 100 is valid")
eq(r.windows[4].display, "80%", "usage: never multiplied by 100")
eq(r.windows[5].display, "1%", "usage: 0.79 percent rounds to 1%")
eq(r.windows[1].state, "ok", "usage: ok state")

-- out of range / wrong type / NaN / missing: "--", never clamped
r = model.usage_rows({ rec { provider = "p", windows = {
	{ label = "a", used = 100.5 }, { label = "b", used = -1 }, { label = "c", used = 0 / 0 },
	{ label = "d", used = "50" }, { label = "e", used = math.huge }, { label = "f" },
} } }, NOW)[1]
for i = 1, 6 do
	eq(r.windows[i].display, "--", "usage: invalid used " .. i)
	eq(r.windows[i].state, "unavailable", "usage: invalid used state " .. i)
	eq(r.windows[i].used, nil, "usage: invalid used dropped " .. i)
end

-- stale by age: valid readings kept only as "~N%"
r = model.usage_rows({ rec { provider = "p", fetched_at = NOW - 300001, windows = {
	{ label = "a", used = 50 }, { label = "b" },
} } }, NOW)[1]
eq(r.stale, true, "usage: old is stale")
eq(r.stale_reason, "old", "usage: stale reason old")
eq(r.windows[1].display, "~50%", "usage: stale keeps valid as ~N%")
eq(r.windows[2].display, "--", "usage: stale without used is --")
eq(model.usage_rows({ rec { provider = "p", fetched_at = NOW - 300000, windows = { { used = 50 } } } }, NOW)[1].windows[1].display,
	"50%", "usage: staleness boundary fresh")

-- future timestamps: past the 60 s grace is stale
eq(model.usage_rows({ rec { provider = "p", fetched_at = NOW + 60001, windows = { { used = 50 } } } }, NOW)[1].stale_reason,
	"future", "usage: future timestamp stale")
eq(model.usage_rows({ rec { provider = "p", fetched_at = NOW + 60000, windows = { { used = 50 } } } }, NOW)[1].windows[1].display,
	"50%", "usage: future boundary fresh")

-- fetched_at absent or not a finite epoch-ms: stale, raw kept as evidence
r = model.usage_rows({ rec({ provider = "p", windows = { { used = 50 } } }, false) }, NOW)[1]
eq(r.stale_reason, "unfetched", "usage: absent fetched_at")
eq(r.windows[1].display, "~50%", "usage: absent fetched_at keeps ~N%")
r = model.usage_rows({ rec { provider = "p", fetched_at = "2026-11-13T10:00:00Z", windows = { { used = 50 } } } }, NOW)[1]
eq(r.stale_reason, "bad_timestamp", "usage: ISO string not parsed")
eq(r.fetched_at_raw, "2026-11-13T10:00:00Z", "usage: raw timestamp kept")
r = model.usage_rows({ rec { provider = "p", fetched_at = 0 / 0, windows = { { used = 50 } } } }, NOW)[1]
eq(r.stale_reason, "bad_timestamp", "usage: NaN fetched_at")

-- error: only a nonempty actual value counts
r = model.usage_rows({ rec { provider = "p", error = "rate limited", windows = { { used = 50 } } } }, NOW)[1]
eq(r.stale_reason, "error", "usage: error string stale")
eq(r.error, "rate limited", "usage: error text kept")
eq(r.windows[1].display, "~50%", "usage: errored record keeps ~N%")
eq(model.usage_rows({ rec { provider = "p", error = "", windows = { { used = 50 } } } }, NOW)[1].windows[1].display,
	"50%", "usage: empty error ignored")
eq(model.usage_rows({ rec { provider = "p", error = false, windows = { { used = 50 } } } }, NOW)[1].windows[1].display,
	"50%", "usage: false error ignored")
eq(model.usage_rows({ rec { provider = "p", error = 0, windows = { { used = 50 } } } }, NOW)[1].stale_reason,
	"error", "usage: 0 is a present error value")

-- reset_pending: a window past resets_at shows "--" until a fresh
-- reading beyond the reset — never 0%, never an inference
r = model.usage_rows({ rec { provider = "p", windows = {
	{ label = "a", used = 80, resets_at = NOW - 1000 },
	{ label = "b", used = 80, resets_at = NOW + 1000 },
	{ label = "c", used = 80 },
} } }, NOW)[1]
eq(r.windows[1].display, "--", "usage: reset_pending not 0%")
eq(r.windows[1].state, "reset_pending", "usage: reset_pending state")
eq(r.windows[1].used, nil, "usage: reset_pending drops the number")
eq(r.windows[2].display, "80%", "usage: reset still ahead shows used")
eq(r.windows[3].display, "80%", "usage: missing resets_at is fine")
-- the reset boundary dominates staleness: nothing newer can be
-- inferred from an expired window, stale reading or not
r = model.usage_rows({ rec { provider = "p", fetched_at = NOW - 400000, windows = {
	{ used = 80, resets_at = NOW - 1000 },
} } }, NOW)[1]
eq(r.windows[1].display, "--", "usage: stale + past reset is --")
eq(r.windows[1].state, "reset_pending", "usage: stale + past reset state")
eq(r.windows[1].used, nil, "usage: stale + past reset drops the number")
-- and the same dominance when the record carries a real error
r = model.usage_rows({ rec { provider = "p", error = "offline", windows = {
	{ used = 80, resets_at = NOW - 1000 },
} } }, NOW)[1]
eq(r.windows[1].display, "--", "usage: error + past reset is --")
eq(r.windows[1].state, "reset_pending", "usage: error + past reset state")
eq(r.windows[1].used, nil, "usage: error + past reset drops the number")

-- no windows at all: an honest unavailable, not a 0%
r = model.usage_rows({ rec { provider = "p" } }, NOW)[1]
eq(#r.windows, 0, "usage: missing windows empty")
r = model.usage_rows({ rec { provider = "p", windows = "junk" } }, NOW)[1]
eq(#r.windows, 0, "usage: non-table windows empty")

-- duplicate provider+account rows collapse to the first
r = model.usage_rows({
	rec { provider = "p", account = "a", plan = "one", windows = {} },
	rec { provider = "p", account = "a", plan = "two", windows = {} },
}, NOW)
eq(#r, 1, "usage: duplicate accounts deduped")
eq(r[1].plan, "one", "usage: dedupe keeps first")

-- metadata kept on the row (the UI shows provider/plan, never the id)
r = model.usage_rows({ rec { provider = "p", account = "acct-1", plan = "pro", windows = {} } }, NOW)[1]
eq(r.account, "acct-1", "usage: account kept for dedupe")
eq(r.plan, "pro", "usage: plan kept")

-- ── usage_coordinator() ──────────────────────────────────────────────
local c = model.usage_coordinator()
local g1 = c.request()
ok(g1 ~= nil, "coord: first request runs")
eq(c.request(), nil, "coord: in-flight request coalesced")
local applied, again = c.settled(g1)
eq(applied, true, "coord: settled applies")
ok(again ~= nil, "coord: dirty request reruns once")
ok(c.in_flight == true, "coord: rerun still in flight")
eq(c.settled(g1), false, "coord: late answer for old generation ignored")
applied, again = c.settled(again)
eq(applied, true, "coord: rerun applies")
eq(again, nil, "coord: clean after rerun")
ok(c.request() ~= nil, "coord: free for the next request")

-- ── usage_refresher() ────────────────────────────────────────────────
-- The real refresh loop init.luau wires up, driven with a mocked async
-- zeron.usage(): initial read pending -> two refreshes coalesce -> the
-- initial resolves -> exactly ONE follow-up read is dispatched -> that
-- resolves -> a later refresh still works -> a late callback applies
-- nothing. Then the same for a failing read.
local function fake_usage()
	local calls = {}
	local pending
	local api = {}
	api.read = function()
		calls[#calls + 1] = true
		local on_ok, on_err
		local self = {}
		function self:after(ok_cb, err_cb)
			on_ok, on_err = ok_cb, err_cb
			pending = self
			return self
		end
		function self:resolve(v)
			on_ok(v)
		end
		function self:reject(e)
			on_err(e)
		end
		return self
	end
	api.calls = calls
	api.pending = function()
		return pending
	end
	return api
end

local usage_api = fake_usage()
local applied_log = {}
local refresh = model.usage_refresher(usage_api.read, function(ok, value)
	applied_log[#applied_log + 1] = { ok = ok, value = value }
end)

refresh()
eq(#usage_api.calls, 1, "refresher: initial read dispatched")
eq(#applied_log, 0, "refresher: nothing applied while pending")
refresh()
refresh() -- two usage.changed while in flight -> one coalesced rerun
eq(#usage_api.calls, 1, "refresher: in-flight refreshes coalesce")
usage_api.pending():resolve({})
eq(#applied_log, 1, "refresher: initial resolve applied")
eq(applied_log[1].ok, true, "refresher: initial resolve ok")
eq(#usage_api.calls, 2, "refresher: exactly one follow-up dispatched")
usage_api.pending():resolve({})
eq(#applied_log, 2, "refresher: follow-up applied")
eq(#usage_api.calls, 2, "refresher: no extra reads after clean settle")
refresh()
eq(#usage_api.calls, 3, "refresher: later refresh dispatches again")
local stale_pending = usage_api.pending()
refresh() -- dirty while third is in flight
stale_pending:resolve({}) -- a late/duplicate callback on the same gen
stale_pending:resolve({}) -- must not apply twice or re-dispatch
eq(#applied_log, 3, "refresher: late callback applies once")
eq(#usage_api.calls, 4, "refresher: dirty follow-up dispatched once")
usage_api.pending():resolve({})
eq(#applied_log, 4, "refresher: final follow-up applied")

-- failure path: a rejected read applies ok=false, still reruns dirty
local usage_api2 = fake_usage()
local applied2 = {}
local refresh2 = model.usage_refresher(usage_api2.read, function(ok, value)
	applied2[#applied2 + 1] = { ok = ok, value = value }
end)
refresh2()
refresh2() -- dirty
usage_api2.pending():reject("denied")
eq(#applied2, 1, "refresher: failure applied once")
eq(applied2[1].ok, false, "refresher: failure ok=false")
eq(applied2[1].value, "denied", "refresher: failure text passed through")
eq(#usage_api2.calls, 2, "refresher: failure still dispatches dirty rerun")
usage_api2.pending():resolve({})
eq(#applied2, 2, "refresher: recovery applies after failure")
eq(applied2[2].ok, true, "refresher: recovered ok=true")

-- synchronous throw: read() raising before returning a promise must
-- still settle the generation, or every later refresh stalls
local throws = true
local reads3 = 0
local applied3 = {}
local pending3
local refresh3 = model.usage_refresher(function()
	reads3 = reads3 + 1
	if throws then
		error("sync boom")
	end
	local self = {}
	function self:after(ok_cb)
		pending3 = ok_cb
		return self
	end
	return self
end, function(ok, value)
	applied3[#applied3 + 1] = { ok = ok, value = value }
end)
refresh3()
eq(#applied3, 1, "refresher: sync throw applied")
eq(applied3[1].ok, false, "refresher: sync throw ok=false")
ok(tostring(applied3[1].value):find("sync boom", 1, true) ~= nil, "refresher: sync throw text")
throws = false
refresh3()
eq(reads3, 2, "refresher: not wedged after sync throw")
pending3({})
eq(#applied3, 2, "refresher: recovery after sync throw")
eq(applied3[2].ok, true, "refresher: recovered after sync throw")

-- ── desktop commands ─────────────────────────────────────────────────
-- the contract, verbatim; nothing runs on load or on a timer — only an
-- explicit click reaches zeron.run (structural: this module never
-- touches zeron/ui globals, which don't even exist here)
eq(table.concat(model.DESKTOP_COMMANDS.start, " "), "--launch", "cmd: start")
eq(table.concat(model.DESKTOP_COMMANDS.pause, " "), "--pause", "cmd: pause")
eq(table.concat(model.DESKTOP_COMMANDS.resume, " "), "--resume", "cmd: resume")
eq(table.concat(model.DESKTOP_COMMANDS.live, " "), "--live", "cmd: live")
eq(table.concat(model.DESKTOP_COMMANDS.restore_clicks, " "), "--click-through off", "cmd: restore clicks")
eq(table.concat(model.DESKTOP_COMMANDS.stop, " "), "--quit", "cmd: stop")
eq(table.concat(model.DESKTOP_COMMANDS.status, " "), "--status", "cmd: status")
eq(model.DESKTOP_TIMEOUT, 12, "cmd: timeout")

-- ── command_runner() ─────────────────────────────────────────────────
-- The "one command at a time" lock lives in the runner itself, so a
-- page/language rebuild (which recreates the buttons) can't lose it:
-- the second click reaches the same runner and is refused.
local function fake_cmd()
	local calls, pending = {}, nil
	local api = {}
	api.run = function(action)
		calls[#calls + 1] = action
		local on_ok, on_err
		local self = {}
		function self:after(ok_cb, err_cb)
			on_ok, on_err = ok_cb, err_cb
			pending = self
			return self
		end
		function self:resolve(v)
			on_ok(v)
		end
		function self:reject(e)
			on_err(e)
		end
		return self
	end
	api.calls = calls
	api.pending = function()
		return pending
	end
	return api
end

local cmd = fake_cmd()
local states = {}
local run = model.command_runner(cmd.run, function(state)
	states[#states + 1] = state
end)
eq(run("status"), true, "runner: first click dispatches")
eq(#cmd.calls, 1, "runner: one run issued")
eq(states[1].kind, "running", "runner: running applied")
eq(states[1].action, "status", "runner: action carried")
eq(run("stop"), false, "runner: second click refused while busy")
eq(#cmd.calls, 1, "runner: no second run while busy")
eq(#states, 1, "runner: refused click applies nothing")
cmd.pending():resolve({ status = 0, stdout = "ok" })
eq(#states, 2, "runner: done applied")
eq(states[2].kind, "done", "runner: done kind")
eq(states[2].result.stdout, "ok", "runner: result passed through")
eq(run("pause"), true, "runner: free again after done")
eq(#cmd.calls, 2, "runner: next run issued")
cmd.pending():reject("denied")
eq(states[#states].kind, "refused", "runner: refused kind")
eq(states[#states].err, "denied", "runner: error passed through")
eq(run("status"), true, "runner: free again after refusal")

-- synchronous throw: run() raising before returning a promise must
-- release the lock, or every later click is refused forever — the same
-- stall the usage refresher's sync-throw test covers
local throws4 = true
local pending4
local states4 = {}
local run4 = model.command_runner(function()
	if throws4 then
		error("sync boom")
	end
	local self = {}
	function self:after(ok_cb)
		pending4 = ok_cb
		return self
	end
	return self
end, function(state)
	states4[#states4 + 1] = state
end)
eq(run4("status"), true, "runner: sync throw still accepts the click")
eq(states4[#states4].kind, "refused", "runner: sync throw applies refused")
ok(tostring(states4[#states4].err):find("sync boom", 1, true) ~= nil, "runner: sync throw error text")
throws4 = false
eq(run4("pause"), true, "runner: lock released after sync throw")
pending4({ status = 0 })
eq(states4[#states4].kind, "done", "runner: recovery after sync throw")

-- ── sorted_sessions() ────────────────────────────────────────────────
local list = model.sorted_sessions({
	x = { id = "x", updated = 10 },
	y = { id = "y", updated = 30 },
	z = { id = "z" },
})
eq(list[1].id, "y", "sessions: freshest first")
eq(list[3].id, "z", "sessions: missing updated last")
eq(#model.sorted_sessions(nil), 0, "sessions: nil empty")
eq(#model.sorted_sessions({ a = "junk" }), 0, "sessions: non-records dropped")

-- ── provider_name() ──────────────────────────────────────────────────
eq(model.provider_name("codex"), "Codex", "name: codex")
eq(model.provider_name("devin"), "Devin", "name: devin")
eq(model.provider_name("opencode"), "OpenCode", "name: opencode")
eq(model.provider_name("claude-code"), "Claude Code", "name: claude-code")
eq(model.provider_name("future-provider"), "future-provider", "name: unknown is itself")
eq(model.provider_name(""), "", "name: empty is itself")

-- ── visible_usage_rows() ─────────────────────────────────────────────
local function urow(provider, account)
	return { provider = provider, account = account, windows = { { label = "w", display = "50%", state = "ok" } } }
end

-- auto keeps present rows, adds nothing for absent providers
local v = model.visible_usage_rows({ urow("codex") }, {})
eq(#v, 1, "vis: auto keeps present row")
eq(v[1].provider, "codex", "vis: auto row provider")
eq(#model.visible_usage_rows({}, {}), 0, "vis: auto absent adds nothing")
eq(#model.visible_usage_rows(nil, nil), 0, "vis: nil rows")

-- hide drops the provider's rows even when active rows exist
v = model.visible_usage_rows({ urow("codex"), urow("devin") }, { codex = "hide" })
eq(#v, 1, "vis: hide drops one")
eq(v[1].provider, "devin", "vis: hide keeps the rest")
eq(#model.visible_usage_rows({ urow("codex") }, { codex = "hide" }), 0, "vis: hide only row")

-- show keeps present rows as-is, no placeholder on top
v = model.visible_usage_rows({ urow("codex") }, { codex = "show" })
eq(#v, 1, "vis: show present keeps one row")
eq(v[1].unavailable, nil, "vis: show present stays real")

-- show on an absent provider adds exactly one honest placeholder:
-- no number, no percentage, no account, no plan
v = model.visible_usage_rows({ urow("devin") }, { codex = "show" })
eq(#v, 2, "vis: show absent appends placeholder")
local ph = v[2]
eq(ph.provider, "codex", "vis: placeholder provider")
eq(ph.unavailable, true, "vis: placeholder marked unavailable")
eq(type(ph.windows), "table", "vis: placeholder windows table")
eq(#ph.windows, 0, "vis: placeholder empty windows")
eq(ph.used, nil, "vis: placeholder no used")
eq(ph.percent, nil, "vis: placeholder no percent")
eq(ph.account, nil, "vis: placeholder no account")
eq(ph.plan, nil, "vis: placeholder no plan")
eq(ph.display, nil, "vis: placeholder no display")
eq(model.provider_name(ph.provider), "Codex", "vis: placeholder names friendly")

-- unknown/future providers are retained under auto
v = model.visible_usage_rows({ urow("gemini") }, {})
eq(#v, 1, "vis: unknown provider retained")
eq(v[1].provider, "gemini", "vis: unknown provider id kept")
-- and aren't placeholders: a managed provider absent stays absent
v = model.visible_usage_rows({ urow("gemini") }, { codex = "auto" })
eq(#v, 1, "vis: unknown only, managed auto adds nothing")

-- invalid or missing preference values count as auto
v = model.visible_usage_rows({ urow("codex") }, { codex = "bogus" })
eq(#v, 1, "vis: bogus mode is auto")
eq(#model.visible_usage_rows({}, { codex = "bogus" }), 0, "vis: bogus mode adds nothing")
v = model.visible_usage_rows({}, { codex = "show", devin = "bogus" })
eq(#v, 1, "vis: mixed bogus still show codex")
eq(v[1].provider, "codex", "vis: bogus devin not shown")
eq(#model.visible_usage_rows({ urow("codex") }, "junk"), 1, "vis: non-table prefs is auto")

-- multiple active accounts for one provider are preserved
v = model.visible_usage_rows({ urow("codex", "a"), urow("codex", "b") }, {})
eq(#v, 2, "vis: two accounts kept")
eq(v[1].account .. "/" .. v[2].account, "a/b", "vis: account order kept")
-- show placeholder logic still fires only when NONE are present
v = model.visible_usage_rows({ urow("codex", "a"), urow("codex", "b") }, { codex = "show" })
eq(#v, 2, "vis: show with accounts adds no placeholder")

-- input rows are never mutated
local original = { urow("codex"), urow("devin") }
local snapshot = { provider = original[1].provider, unavailable = original[1].unavailable, windows = original[1].windows }
model.visible_usage_rows(original, { codex = "hide", devin = "show" })
eq(original[1].provider, snapshot.provider, "vis: input row provider unchanged")
eq(original[1].unavailable, snapshot.unavailable, "vis: input row not marked")
ok(original[1].windows == snapshot.windows, "vis: input windows table untouched")
eq(#original, 2, "vis: input list untouched")

print(string.format("%d checks, %d failures", checks, failures))
if failures > 0 then
	os.exit(1)
end
print("OK")

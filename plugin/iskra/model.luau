-- Iskra's pure logic: chat statuses, their worst-case aggregation, the
-- animation each status maps to, usage-window rows, the usage refresh
-- coordinator and the desktop pet's command table.
--
-- This file is Lua 5.4-clean on purpose — no Luau type annotations, no
-- `if` expressions, no `+=`, no generalized iteration — so it runs under
-- the plain `lua` interpreter: plugin/iskra/tests/model_test.lua.

local M = {}

-- Worst first: the rank each status takes when chats are merged.
M.PRECEDENCE = { "awaitingInput", "errored", "working", "completed", "unknown", "idle" }

local RANK = {}
for i, key in ipairs(M.PRECEDENCE) do
	RANK[key] = i
end

local KNOWN = {
	awaitingInput = true,
	errored = true,
	working = true,
	completed = true,
	idle = true,
}

-- One session's status key: awaitingInput, errored, working, completed,
-- unknown or idle. Anything off that list, a stale flag or a non-record
-- reads as unknown — the picture stays truthful instead of guessing.
function M.status(session)
	if type(session) ~= "table" then
		return "unknown"
	end
	if session.statusStale == true or session.status_stale == true then
		return "unknown"
	end
	if KNOWN[session.status] then
		return session.status
	end
	return "unknown"
end

-- The worst status key across a map of sessions (empty map: idle — the
-- caller says "No chats", the map itself invents no connection state).
-- lastTurnOutcome is history, never consulted.
function M.aggregate(sessions)
	local worst = "idle"
	if type(sessions) ~= "table" then
		return worst
	end
	for _, session in pairs(sessions) do
		local key = M.status(session)
		if RANK[key] < RANK[worst] then
			worst = key
		end
	end
	return worst
end

-- The standing animation each status draws (v6 art; no floor_/wall_
-- poses in the plugin widget). Note: there is no standing_subdued in the
-- v6 manifest — unknown stands idle, and the muted caption carries the
-- truth that the status couldn't be read.
M.ANIM = {
	working = "standing_focus_laptop",
	awaitingInput = "standing_question",
	errored = "standing_sadness",
	completed = "standing_joy",
	unknown = "standing_idle",
	idle = "standing_idle",
}
M.WORKING_ALT = "standing_focus_solder"
M.WORKING_ALT_MS = 12000

function M.anim_key(status_key, alternate)
	if status_key == "working" and alternate == true then
		return M.WORKING_ALT
	end
	return M.ANIM[status_key] or M.ANIM.unknown
end

-- ── usage ────────────────────────────────────────────────────────────
-- `used` is already a percent from the SDK (never multiplied). A record
-- is stale when it carries a real error or its fetched_at isn't a finite
-- epoch-ms within the fresh band; stale windows keep a valid reading as
-- "~N%" and everything else as "--". A window past resets_at is
-- reset_pending ("--"), never 0% — a reset can't be inferred backwards.

M.STALE_MS = 300000 -- a reading older than 5 minutes is stale
M.FUTURE_MS = 60000 -- …or more than a minute in the future

local function finite(n)
	return type(n) == "number" and n == n and n ~= math.huge and n ~= -math.huge
end
M.finite = finite

local function valid_used(v)
	return finite(v) and v >= 0 and v <= 100
end

-- A real error is a nonempty value: not nil, not false, not "".
local function has_error(e)
	return e ~= nil and e ~= false and e ~= ""
end
M.has_error = has_error

-- Why a record is stale, or nil when it's fresh. fetched_at counts only
-- as a finite number of epoch milliseconds — no string or unit guessing.
local function stale_reason(r, now_ms)
	if has_error(r.error) then
		return "error"
	end
	local f = r.fetched_at
	if f == nil then
		return "unfetched"
	end
	if not finite(f) or not finite(now_ms) then
		return "bad_timestamp"
	end
	if now_ms - f > M.STALE_MS then
		return "old"
	end
	if f - now_ms > M.FUTURE_MS then
		return "future"
	end
	return nil
end
M.stale_reason = stale_reason

local function round_used(v)
	return math.floor(v + 0.5)
end

-- One window's display: { label, display, state, used?, resets_at? }.
-- state: "ok" | "reset_pending" | "unavailable" | "stale". A window past
-- its resets_at is reset_pending first — the reset boundary dominates
-- stale/error bookkeeping, since nothing newer can be inferred.
function M.usage_window(w, stale, now_ms)
	if type(w) ~= "table" then
		w = {}
	end
	local used_ok = valid_used(w.used)
	local resets = finite(w.resets_at) and w.resets_at or nil
	local display, state, used
	if resets ~= nil and finite(now_ms) and resets <= now_ms then
		state, display = "reset_pending", "--"
	elseif stale then
		state = "stale"
		if used_ok then
			used = round_used(w.used)
			display = "~" .. used .. "%"
		else
			display = "--"
		end
	elseif used_ok then
		used = round_used(w.used)
		state, display = "ok", used .. "%"
	else
		state, display = "unavailable", "--"
	end
	return {
		label = type(w.label) == "string" and w.label or nil,
		display = display,
		state = state,
		used = used,
		resets_at = resets,
	}
end

-- Active records only, sorted by provider then account, deduplicated by
-- provider+account. Each row: { provider, account?, plan?, stale,
-- stale_reason?, error?, fetched_at?, fetched_at_raw, windows }. The raw
-- fetched_at is kept as evidence of the real schema when it isn't a
-- finite epoch-ms (shown as-is, never parsed into a guess).
function M.usage_rows(records, now_ms)
	local rows = {}
	if type(records) ~= "table" then
		return rows
	end
	for _, r in ipairs(records) do
		if type(r) == "table" and r.active == true then
			local stale = stale_reason(r, now_ms)
			local windows = {}
			if type(r.windows) == "table" then
				for _, w in ipairs(r.windows) do
					table.insert(windows, M.usage_window(w, stale ~= nil, now_ms))
				end
			end
			table.insert(rows, {
				provider = type(r.provider) == "string" and r.provider or "?",
				account = type(r.account) == "string" and r.account or nil,
				plan = type(r.plan) == "string" and r.plan or nil,
				stale = stale ~= nil,
				stale_reason = stale,
				error = has_error(r.error) and tostring(r.error) or nil,
				fetched_at = finite(r.fetched_at) and r.fetched_at or nil,
				fetched_at_raw = r.fetched_at,
				windows = windows,
			})
		end
	end
	table.sort(rows, function(a, b)
		if a.provider ~= b.provider then
			return a.provider < b.provider
		end
		return (a.account or "") < (b.account or "")
	end)
	local seen, out = {}, {}
	for _, row in ipairs(rows) do
		local key = row.provider .. "\0" .. (row.account or "")
		if not seen[key] then
			seen[key] = true
			table.insert(out, row)
		end
	end
	return out
end

-- The refresh loop itself, with the read and the apply injected so it
-- runs under a mock in tests exactly as init.luau runs it live:
--   read(): an object with :after(on_ok, on_err) (zeron.usage()'s promise)
--   apply(ok, value): ok=true + the records, or ok=false + error text
-- Returns the refresh function: each call asks the coordinator for a
-- generation and dispatches that read; callbacks dispatch an allocated
-- follow-up generation directly (never a second request() for the same
-- token — that would only mark dirty and stall the queue).
function M.usage_refresher(read, apply)
	local coord = M.usage_coordinator()
	local dispatch
	dispatch = function(gen)
		-- A synchronous read() throw must still settle the generation —
		-- otherwise in_flight stays true and refreshes stall forever.
		local ok, promise = pcall(read)
		if not ok then
			local applied, again = coord.settled(gen)
			if applied then
				apply(false, tostring(promise))
				if again then
					dispatch(again)
				end
			end
			return
		end
		promise:after(function(records)
			local applied, again = coord.settled(gen)
			if not applied then
				return
			end
			apply(true, records)
			if again then
				dispatch(again)
			end
		end, function(err)
			local applied, again = coord.settled(gen)
			if not applied then
				return
			end
			apply(false, tostring(err))
			if again then
				dispatch(again)
			end
		end)
	end
	return function()
		local gen = coord.request()
		if gen then
			dispatch(gen)
		end
	end
end

-- Coalesced re-reads of the cached zeron.usage(): a request while one is
-- in flight is remembered (dirty) and runs once when it settles, never
-- stacked. A late answer for an old generation applies nothing.
function M.usage_coordinator()
	local c = { generation = 0, in_flight = false, dirty = false }
	function c.request()
		if c.in_flight then
			c.dirty = true
			return nil
		end
		c.in_flight = true
		c.generation = c.generation + 1
		return c.generation
	end
	function c.settled(gen)
		if gen ~= c.generation then
			return false, nil
		end
		c.in_flight = false
		if c.dirty then
			c.dirty = false
			return true, c.request()
		end
		return true, nil
	end
	return c
end

-- Sessions as a list for the page, freshest first (missing timestamps
-- sink to the end, ties break by id so the order doesn't jitter).
function M.sorted_sessions(sessions)
	local out = {}
	if type(sessions) ~= "table" then
		return out
	end
	for _, s in pairs(sessions) do
		if type(s) == "table" then
			table.insert(out, s)
		end
	end
	table.sort(out, function(a, b)
		local au = type(a.updated) == "number" and a.updated or 0
		local bu = type(b.updated) == "number" and b.updated or 0
		if au ~= bu then
			return au > bu
		end
		return tostring(a.id) < tostring(b.id)
	end)
	return out
end

-- The canonical provider ids Iskra's usage_* settings manage, in the
-- order forced-show placeholders are appended.
M.USAGE_PROVIDERS = { "codex", "devin", "opencode", "claude-code" }

-- Friendly display names for those ids; any other provider id (current
-- or future) displays as itself.
M.PROVIDER_NAMES = {
	codex = "Codex",
	devin = "Devin",
	opencode = "OpenCode",
	["claude-code"] = "Claude Code",
}

function M.provider_name(id)
	return M.PROVIDER_NAMES[id] or id
end

local VISIBILITY_MODES = { auto = true, show = true, hide = true }

-- visible_usage_rows(rows, preferences): pure presentation filter applied
-- to the already-normalized usage_rows output. preferences maps a
-- canonical provider id to "auto" | "show" | "hide"; a missing or
-- unrecognized value counts as auto.
--   auto  — keep the provider's rows if present, add nothing when absent
--   hide  — drop the provider's rows
--   show  — keep present rows; when absent, append exactly one
--           unavailable placeholder { provider, windows = {},
--           unavailable = true } — no fabricated number, percentage,
--           account or plan
-- Rows for providers with no preference entry (unknown/future providers)
-- pass through untouched. Input rows are never mutated.
function M.visible_usage_rows(rows, preferences)
	local prefs = type(preferences) == "table" and preferences or {}
	local function mode(provider)
		local m = prefs[provider]
		if VISIBILITY_MODES[m] then
			return m
		end
		return "auto"
	end
	local out = {}
	local present = {}
	for _, row in ipairs(rows or {}) do
		local provider = row and row.provider
		if provider ~= nil and mode(provider) ~= "hide" then
			out[#out + 1] = row
			present[provider] = true
		end
	end
	for _, id in ipairs(M.USAGE_PROVIDERS) do
		if not present[id] and mode(id) == "show" then
			out[#out + 1] = { provider = id, windows = {}, unavailable = true }
		end
	end
	return out
end

-- The desktop helper's commands, run only on an explicit click — never
-- on load, on a timer or on a probe. Args go to zeron-pet as given.
M.DESKTOP_COMMANDS = {
	start = { "--launch" },
	pause = { "--pause" },
	resume = { "--resume" },
	live = { "--live" },
	restore_clicks = { "--click-through", "off" },
	stop = { "--quit" },
	status = { "--status" },
}
M.DESKTOP_TIMEOUT = 12

-- command_runner(run, apply): the desktop buttons' click guard. The
-- "one command at a time" lock lives inside the runner itself, so it
-- survives UI rebuilds (a language change, a page remount): a second
-- click while a command is in flight can't issue a second run.
-- `run(action)` must return a promise with :after(ok, err);
-- `apply(state)` receives { kind = "running" | "done" | "refused",
-- action, result? | err? } — the UI mirrors it into reactive state.
function M.command_runner(run, apply)
	local pending = false
	return function(action)
		if pending then
			return false
		end
		pending = true
		apply({ kind = "running", action = action })
		-- A synchronous run() throw (or a non-promise return) must release
		-- the lock too, or every later click is refused forever — the same
		-- stall class the usage refresher guards against.
		local ok, err = pcall(function()
			run(action):after(function(result)
				pending = false
				apply({ kind = "done", action = action, result = result })
			end, function(run_err)
				pending = false
				apply({ kind = "refused", action = action, err = run_err })
			end)
		end)
		if not ok then
			pending = false
			apply({ kind = "refused", action = action, err = err })
		end
		return true
	end
end

-- Wall-clock epoch milliseconds, for fetched_at/resets_at comparisons.
function M.now_ms()
	return math.floor(os.time() * 1000)
end

return M

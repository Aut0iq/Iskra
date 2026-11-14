-- Tests for plugin/iskra/i18n.luau, run under plain Lua 5.4+:
--   lua plugin/iskra/tests/i18n_test.lua        (from the repo root)
--   lua tests/i18n_test.lua                     (from plugin/iskra)
-- i18n.luau is intentionally Lua 5.4-clean so this runs without Luau.

local here = (arg and arg[0] or ""):match("^(.*)[/\\][^/\\]*$") or "."
local i18n = dofile(here .. "/../i18n.luau")

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

-- A mutable-box stand-in for the host's reactive shared state/settings.
local function box(value)
	return {
		get = function()
			return value
		end,
		set = function(v)
			value = v
		end,
	}
end

local catalogs = {
	en = {
		["status.working"] = "Working",
		["usage.read_error"] = "Couldn't read usage: {why}",
		["only.en"] = "English only",
	},
	ru = {
		["status.working"] = "Работает",
		["usage.read_error"] = "Не удалось прочитать использование: {why}",
	},
	["zh-Hans"] = {
		["status.working"] = "工作中",
	},
}

local function fake_format(msg, args)
	if type(args) ~= "table" then
		return msg
	end
	return (msg:gsub("{(%w+)}", function(k)
		return tostring(args[k])
	end))
end

local function build(sel, host)
	local sel_box = box(sel)
	local host_box = box(host)
	local cat_box = box(catalogs)
	local I = i18n.new(function()
		return cat_box.get()
	end, sel_box.get, host_box.get, fake_format)
	return I, sel_box, host_box, cat_box
end

-- ── normalize() ──────────────────────────────────────────────────────
eq(i18n.normalize("en"), "en", "norm: en")
eq(i18n.normalize("en-US"), "en", "norm: en-US")
eq(i18n.normalize("EN_us"), "en", "norm: EN_us")
eq(i18n.normalize("ru"), "ru", "norm: ru")
eq(i18n.normalize("ru_RU"), "ru", "norm: ru_RU")
eq(i18n.normalize("RU-ru"), "ru", "norm: case")
eq(i18n.normalize("zh-CN"), "zh-Hans", "norm: zh-CN")
eq(i18n.normalize("zh-Hans"), "zh-Hans", "norm: zh-Hans")
eq(i18n.normalize("zh"), "zh-Hans", "norm: bare zh -> zh-Hans")
eq(i18n.normalize("zh-hans-CN"), "zh-Hans", "norm: zh-hans-CN")
eq(i18n.normalize("zh-Hant"), "en", "norm: zh-Hant unsupported -> en")
eq(i18n.normalize("zh-TW"), "en", "norm: zh-TW unsupported -> en")
eq(i18n.normalize("de"), "en", "norm: unknown -> en")
eq(i18n.normalize(nil), "en", "norm: nil")
eq(i18n.normalize(42), "en", "norm: non-string")

-- ── language(): auto follows the (mutable) host ─────────────────────
local I, sel_box, host_box = build("auto", "en")
eq(I.language(), "en", "lang: auto -> en host")
eq(I.s("status.working"), "Working", "auto: en message")
host_box.set("ru")
eq(I.language(), "ru", "lang: auto follows host -> ru")
eq(I.s("status.working"), "Работает", "auto: ru message")
host_box.set("zh-Hans")
eq(I.language(), "zh-Hans", "lang: auto follows host -> zh-Hans")
eq(I.s("status.working"), "工作中", "auto: zh-Hans message")
host_box.set("ru_RU")
eq(I.language(), "ru", "lang: host ru_RU normalizes")
host_box.set("de")
eq(I.language(), "en", "lang: unsupported host -> en")
host_box.set(nil)
eq(I.language(), "en", "lang: nil host -> en")

-- ── manual selection overrides the host ─────────────────────────────
I, sel_box, host_box = build("en", "ru")
eq(I.language(), "en", "lang: manual en over ru host")
eq(I.s("status.working"), "Working", "manual: en over ru")
sel_box.set("ru")
eq(I.language(), "ru", "lang: manual ru")
eq(I.s("status.working"), "Работает", "manual: ru message")
sel_box.set("zh-Hans")
eq(I.language(), "zh-Hans", "lang: manual zh-Hans")
eq(I.s("status.working"), "工作中", "manual: zh-Hans message")
-- a non-catalog selection value falls back to auto (host)
sel_box.set("de")
eq(I.language(), "ru", "lang: invalid selection -> auto host")
sel_box.set("zh-CN")
eq(I.language(), "ru", "lang: zh-CN is not a catalog id -> auto host")

-- ── per-key fallback: chosen locale -> English -> key ────────────────
I = build("ru", "en")
eq(I.s("only.en"), "English only", "fallback: missing in ru -> en")
I = build("zh-Hans", "en")
eq(I.s("usage.read_error", { why = "x" }), "Couldn't read usage: x", "fallback: zh catalog misses key -> en")
eq(I.s("no.such.key"), "no.such.key", "fallback: missing everywhere -> key")

-- ── interpolation goes through the injected formatter ───────────────
I = build("auto", "ru")
eq(I.s("usage.read_error", { why = "denied" }), "Не удалось прочитать использование: denied", "format: ru args")
I = build("auto", "en")
eq(I.s("usage.read_error", { why = "nope" }), "Couldn't read usage: nope", "format: en args")
-- no formatter: the raw message passes through
I = i18n.new(function()
	return catalogs
end, function()
	return "auto"
end, function()
	return "en"
end)
eq(I.s("usage.read_error", { why = "x" }), "Couldn't read usage: {why}", "format: absent formatter passthrough")

-- ── t() returns a closure that re-reads the live language ───────────
I, sel_box, host_box = build("auto", "en")
local caption = I.t("status.working")
eq(caption(), "Working", "t: en value")
host_box.set("ru")
eq(caption(), "Работает", "t: follows host to ru")
sel_box.set("zh-Hans")
eq(caption(), "工作中", "t: follows manual zh-Hans")

print(string.format("%d checks, %d failures", checks, failures))
if failures > 0 then
	os.exit(1)
end
print("OK")

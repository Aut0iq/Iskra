-- Iskra — plugin-owned localization, pure Lua 5.4 (also parses under Luau).
--
-- init.luau builds one instance over the host's locale seams:
--   M.new(get_catalogs, get_selection, get_host, format) -> {
--       s(key, args)        -> string   (immediate value)
--       t(key, args)        -> function -> string (reactive closure)
--       language()          -> effective plugin language id
--   }
--   get_catalogs()  -> { [locale] = { [key] = message } }
--                      (the flat per-locale catalogs the host keeps in
--                      ui.shared("plugin.iskra.locales", {}))
--   get_selection() -> the "language" setting: "auto" | "en" | "ru" |
--                      "zh-Hans" (anything else counts as auto)
--   get_host()      -> Zeron's effective language, e.g. "ru", "zh-Hans",
--                      "ru_RU", "en-US"
--   format(msg,args) -> the host's ICU formatter (@ds/locale.format);
--                      may be nil — messages then pass through as-is
--
-- Resolution: a manual selection wins; "auto" follows the host language.
-- Catalog lookup falls back chosen locale -> English -> the key itself.
-- This module never writes to any shared state and never changes Zeron's
-- own language — it only picks which of the plugin's catalogs to read.

local M = {}

-- The language ids Iskra ships catalogs for; a "language" setting value
-- is a manual override only when it is exactly one of these.
local MANUAL = { en = true, ru = true, ["zh-Hans"] = true }

-- normalize(locale) -> "en" | "ru" | "zh-Hans"
-- en* -> en, ru* -> ru, zh-Hans/zh-CN/zh -> zh-Hans (the Simplified
-- Chinese catalog); anything else — including zh-Hant/zh-TW, for which
-- there is no catalog — falls back to English.
local function normalize(locale)
	if type(locale) ~= "string" then
		return "en"
	end
	local l = string.lower(locale):gsub("_", "-")
	if l == "en" or string.match(l, "^en%-") then
		return "en"
	end
	if l == "ru" or string.match(l, "^ru%-") then
		return "ru"
	end
	if l == "zh" or l == "zh-hans" or l == "zh-cn" or string.match(l, "^zh%-hans%-") or string.match(l, "^zh%-cn%-") then
		return "zh-Hans"
	end
	return "en"
end

M.normalize = normalize

function M.new(get_catalogs, get_selection, get_host, format)
	local fmt = type(format) == "function" and format
		or function(msg)
			return msg
		end

	local function language()
		local sel = type(get_selection) == "function" and get_selection() or nil
		if type(sel) == "string" and MANUAL[sel] then
			return sel
		end
		local host = type(get_host) == "function" and get_host() or nil
		return normalize(host)
	end

	local function s(key, args)
		local catalogs = type(get_catalogs) == "function" and get_catalogs() or nil
		if type(catalogs) ~= "table" then
			catalogs = {}
		end
		local lang = language()
		local cat = catalogs[lang]
		local msg = type(cat) == "table" and cat[key] or nil
		if msg == nil then
			local en = catalogs.en
			msg = type(en) == "table" and en[key] or nil
		end
		if type(msg) ~= "string" then
			msg = key
		end
		return fmt(msg, args)
	end

	local function t(key, args)
		return function()
			return s(key, args)
		end
	end

	return { s = s, t = t, language = language }
end

return M

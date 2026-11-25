local zeron = require("./zeron")("iskra")
local model = require("./model")
local i18n_mod = require("./i18n")

-- Iskra
--
-- The Iskra companion, inside Zeron. A small animated Iskra stands
-- by a chat's header mirroring that chat's status; her own sidebar page
-- shows a bigger one for the worst status across chats, the agents' usage
-- as Zeron last read it, the chats (each opening on a click), and — only
-- when the person turns on `desktop_controls` — buttons driving the
-- separate `zeron-pet` helper on this device.
--
-- Her pictures are packaged assets: assets/animations.luau (generated at
-- package time) maps each animation key to { frames = { "assets/v6/…" },
-- frame_ms }. Until the package is built she draws nothing where she
-- would stand — an empty space, never a kaomoji or a borrowed picture.

local function opt(name: string): any
	local ok, m = pcall(require, name)
	if ok then
		return m
	end
	return nil
end
local Button = opt("@ds/button")
local motion = opt("@ds/motion")
local ds_locale = opt("@ds/locale")
local anims = opt("./assets/animations")
if type(anims) ~= "table" then
	anims = {}
end

local show_header = zeron.setting("show_header")
local animate = zeron.setting("animate")
local show_usage = zeron.setting("show_usage")
local desktop_controls = zeron.setting("desktop_controls")
local language_setting = zeron.setting("language")

-- Per-provider visibility, keyed by canonical provider id (the settings
-- are usage_codex / usage_devin / usage_opencode / usage_claude_code).
local usage_visibility = {
	codex = zeron.setting("usage_codex"),
	devin = zeron.setting("usage_devin"),
	opencode = zeron.setting("usage_opencode"),
	["claude-code"] = zeron.setting("usage_claude_code"),
}

-- Iskra's own words. The catalogs are the host's shared per-locale
-- tables, the host language is Zeron's effective one; "auto" follows it.
-- Read-only — this only picks which catalog to read, it never changes
-- Zeron's language or host-owned Settings rows (those stay English).
local locales_box = ui.shared("plugin.iskra.locales", {})
local host_locale_box = ui.shared("zeron.locale", "en")
local i18n = i18n_mod.new(function()
	return locales_box:get()
end, function()
	return language_setting:get()
end, function()
	return host_locale_box:get()
end, ds_locale and ds_locale.format)

-- Read at row-build time so toggling a provider's visibility re-filters
-- the already-normalized rows without another usage read.
local function usage_preferences(): { [string]: any }
	local p: { [string]: any } = {}
	for id, setting in pairs(usage_visibility) do
		p[id] = setting:get()
	end
	return p
end

local STATUS_TONE = {
	awaitingInput = "$warning",
	errored = "$danger",
	working = "$accent",
	completed = "$success",
	unknown = "$text.tertiary",
	idle = "$text.tertiary",
}

local function trim(s: any): string
	if type(s) ~= "string" then
		return ""
	end
	return string.match(s, "^%s*(.-)%s*$") or ""
end

-- Frames hold still when animation is off, when the window is out of
-- sight, or when the person (or the system) asks for reduced motion.
local function anim_paused(): boolean
	if animate:peek() == false then
		return true
	end
	if not zeron.window(true).shown then
		return true
	end
	if motion and motion.reduced_now and motion.reduced_now() then
		return true
	end
	return false
end

-- One animated Iskra at `size`; `status()` reads her status key live.
-- Rebuilt as the animation changes so each plays at its own frame_ms,
-- its timer scoped to it and cleaned up with it. While she works she
-- swaps focus_laptop for focus_solder every 12 s.
local function Pet(props: any): Node
	local alt = ui.use_state(0)
	local stop_alt = ui.every(model.WORKING_ALT_MS, function()
		if not anim_paused() and props.status() == "working" then
			alt:set(alt:peek() + 1)
		end
	end)
	pcall(ui.on_cleanup, stop_alt)
	return ui.presence(function(): string
		return model.anim_key(props.status(), alt:get() % 2 == 1)
	end, function(key: string): Node
		local a = anims[key]
		if type(a) ~= "table" or type(a.frames) ~= "table" or #a.frames == 0 then
			-- Art not packaged yet (or an animation the build lacks):
			-- hold the space, draw nothing.
			return ui.div { w = props.size, h = props.size, shrink = 0 }
		end
		local frame = ui.use_state(0)
		local stop = ui.every(math.max(60, tonumber(a.frame_ms) or 140), function()
			if anim_paused() then
				return
			end
			frame:set((frame:peek() + 1) % #a.frames)
		end)
		pcall(ui.on_cleanup, stop)
		return ui.image {
			src = function(): string
				return a.frames[(frame:get() % #a.frames) + 1]
			end,
			w = props.size,
			h = props.size,
			shrink = 0,
			fit = "contain",
			label = i18n.s("iskra.alt"),
		}
	end)
end

local function status_text(key: any): () -> string
	return function(): string
		return i18n.s("status." .. key())
	end
end

local function status_color(key: any): () -> string
	return function(): string
		return STATUS_TONE[key()] or "$text.secondary"
	end
end

-- ── usage, as Zeron last read it ─────────────────────────────────────
-- Coalesced re-reads of the cached zeron.usage(): on usage.changed, on a
-- 30 s fallback, and when the window comes back into view. Never
-- usage_refresh — a fresh read is the engine's to schedule.

local usage_box = ui.state("iskra.usage", { rows = {}, ready = false, err = nil })

-- The refresh loop is model.usage_refresher — the same code the tests
-- drive with a mock. A failed read clears the rows and leaves the error
-- banner: no normal-looking numbers standing in for a dead read.
local refresh_usage = model.usage_refresher(function(): any
	return zeron.usage()
end, function(ok: boolean, value: any)
	if ok then
		usage_box:set({ rows = model.usage_rows(value, model.now_ms()), ready = true, err = nil })
	else
		usage_box:set({ rows = {}, ready = true, err = tostring(value) })
	end
end)

zeron.on("usage.changed", function()
	refresh_usage()
end)
zeron.every(30, function()
	refresh_usage()
end)
do
	local was = zeron.window(true).shown
	ui.effect(function()
		local shown = zeron.window().shown
		if shown and not was then
			refresh_usage()
		end
		was = shown
	end)
end
refresh_usage()

-- ── the header's Iskra ───────────────────────────────────────────────

zeron.place {
	at = "#session-header",
	after = function(): Node
		local session = zeron.context("session")
		if type(session) ~= "table" or type(session.id) ~= "string" then
			return ui.div {}
		end
		local sid = session.id
		local function key(): string
			return model.status(zeron.context_of("session", sid))
		end
		-- Rebuilt when the effective language changes so static strings
		-- (like the picture's label) don't linger in the old language.
		return ui.presence(function(): string
			return i18n.language()
		end, function(_lang: string): Node
			return ui.show(function(): boolean
				return show_header:get() ~= false
			end, ui.row {
				items = "center",
				gap = "$space.2",
				px = "$space.3",
				py = "$space.1",
				Pet { size = 48, status = key },
				ui.column {
					gap = "$space.1",
					ui.text { status_text(key), text_size = "$font.size.sm", weight = "$font.weight.medium", color = status_color(key) },
					ui.text { i18n.t("iskra.alt"), text_size = "$font.size.xs", color = "$text.tertiary" },
				},
			})
		end)
	end,
}

-- ── her page ─────────────────────────────────────────────────────────

local function Btn(props: any): Node
	if Button then
		return Button(props)
	end
	-- a plain ui.button knows no tone/size/busy: strip them
	local p = table.clone(props)
	p.tone, p.size, p.shape, p.icon, p.busy = nil, nil, nil, nil, nil
	return ui.button(p)
end

local function usage_row(r: any): Node
	local lines: { Node } = {}
	for _, w in r.windows do
		table.insert(lines, ui.row {
			w_full = true,
			items = "center",
			gap = "$space.2",
			ui.text { w.label or "—", text_size = "$font.size.xs", color = "$text.tertiary", grow = 1 },
			ui.text {
				w.display,
				text_size = "$font.size.sm",
				tabular = true,
				color = if w.state == "ok" then "$text.primary" else "$text.tertiary",
			},
		})
	end
	if r.unavailable then
		-- a "Show" placeholder: the provider reports nothing to Zeron,
		-- so there is honestly nothing to measure
		table.insert(lines, ui.text {
			i18n.t("usage.provider_unavailable", { name = model.provider_name(r.provider) }),
			text_size = "$font.size.xs",
			color = "$text.tertiary",
		})
	elseif #r.windows == 0 then
		table.insert(lines, ui.text { i18n.t("usage.no_windows"), text_size = "$font.size.xs", color = "$text.tertiary" })
	end
	local notes: { string } = {}
	if r.stale then
		table.insert(notes, i18n.s("usage.stale"))
	end
	if r.error then
		table.insert(notes, i18n.s("usage.error_prefix") .. r.error)
	end
	local reset = false
	for _, w in r.windows do
		if w.state == "reset_pending" then
			reset = true
		end
	end
	if reset then
		table.insert(notes, i18n.s("usage.reset_pending"))
	end
	if r.fetched_at == nil and r.fetched_at_raw ~= nil then
		-- evidence of the real schema, shown as it came, never parsed
		table.insert(notes, i18n.s("usage.bad_timestamp", { raw = tostring(r.fetched_at_raw) }))
	end
	for _, note in notes do
		table.insert(lines, ui.text { note, text_size = "$font.size.xs", color = "$text.tertiary" })
	end
	local head = model.provider_name(r.provider)
	if r.plan then
		head = `{head} · {r.plan}`
	end
	return ui.column {
		w_full = true,
		gap = "$space.1",
		ui.text { head, text_size = "$font.size.sm", weight = "$font.weight.semibold", color = "$text.primary" },
		table.unpack(lines),
	}
end

-- ui.each over a reactive list of rows, with the computed and plain
-- forms as fallbacks if this zgx build wants one or the other.
local function each_or(list: () -> { any }, row: (any) -> Node, fallback: () -> string): Node
	local ok, node = pcall(ui.each, list, row)
	if ok then
		return node
	end
	ok, node = pcall(function()
		return ui.each(ui.computed(list), row)
	end)
	if ok then
		return node
	end
	return ui.text { fallback, text_size = "$font.size.xs", color = "$text.tertiary" }
end

local function usage_section(): Node
	return ui.column {
		w_full = true,
		gap = "$space.2",
		ui.text { i18n.t("usage.title"), text_size = "$font.size.sm", weight = "$font.weight.semibold", color = "$text.secondary" },
		ui.text {
			function(): string
				local err = (usage_box:get() or {}).err
				return if err then i18n.s("usage.read_error", { why = err }) else ""
			end,
			text_size = "$font.size.xs",
			color = "$danger",
		},
		ui.show(function(): boolean
			local box = usage_box:get() or {}
			-- A failed read shows the error line, not "no providers".
			return box.err == nil and #model.visible_usage_rows(box.rows or {}, usage_preferences()) == 0
		end, ui.text { i18n.t("usage.empty"), text_size = "$font.size.xs", color = "$text.tertiary" }),
		each_or(function(): { any }
			local box = usage_box:get() or {}
			return model.visible_usage_rows(box.rows or {}, usage_preferences())
		end, usage_row, i18n.t("usage.empty")),
		-- coverage honesty: only what Zeron itself reads appears —
		-- an absent provider is unsupported or quiet, never invented
		ui.text { i18n.t("usage.note"), text_size = "$font.size.xs", color = "$text.tertiary" },
	}
end

local function chat_row(s: any): Node
	local sid = s.id
	local function key(): string
		return model.status(zeron.context_of("session", sid))
	end
	return ui.row {
		w_full = true,
		items = "center",
		gap = "$space.2",
		ui.column {
			grow = 1,
			gap = "$space.1",
			ui.text { s.title or "…", text_size = "$font.size.sm", weight = "$font.weight.medium", color = "$text.primary" },
			ui.text { status_text(key), text_size = "$font.size.xs", color = status_color(key) },
		},
		Btn { i18n.t("chats.open"), size = "xs", tone = "ghost", on_click = function()
			zeron.open({ session = sid })
		end },
	}
end

local function chats_section(): Node
	return ui.column {
		w_full = true,
		gap = "$space.2",
		ui.text { i18n.t("chats.title"), text_size = "$font.size.sm", weight = "$font.weight.semibold", color = "$text.secondary" },
		ui.show(function(): boolean
			return next(zeron.sessions()) == nil
		end, ui.text { i18n.t("chats.empty"), text_size = "$font.size.xs", color = "$text.tertiary" }),
		each_or(function(): { any }
			return model.sorted_sessions(zeron.sessions())
		end, chat_row, i18n.t("chats.empty")),
	}
end

-- The desktop helper's controls: only when the person turns the setting
-- on, and every button runs `zeron-pet` only on its click — no probing,
-- no autoboot, nothing on a timer.
local DESKTOP_ACTIONS = { "start", "pause", "resume", "live", "restore_clicks", "stop", "status" }

-- The buttons' pending/result live at plugin level, not inside the
-- section: a UI rebuild (a language change, a page remount) must not
-- lose a running command's lock or its result. model.command_runner
-- holds the lock itself; these boxes just mirror it for the reactive UI.
local desktop_pending_box = ui.state("iskra.desktop.pending", false)
local desktop_result_box = ui.state("iskra.desktop.result", nil :: any)

local desktop_run = model.command_runner(function(action: string): any
	return zeron.run("zeron-pet", model.DESKTOP_COMMANDS[action], { timeout = model.DESKTOP_TIMEOUT })
end, function(state: any)
	desktop_pending_box:set(state.kind == "running")
	if state.kind == "done" then
		local r = if type(state.result) == "table" then state.result else nil
		desktop_result_box:set({
			kind = "done",
			action = state.action,
			status = r and r.status,
			stdout = r and r.stdout,
			stderr = r and r.stderr,
		})
	else
		local st: any = { kind = state.kind, action = state.action }
		if state.err ~= nil then
			st.err = tostring(state.err)
		end
		desktop_result_box:set(st)
	end
end)

local function desktop_section(): Node
	local function run(action: string)
		if not model.DESKTOP_COMMANDS[action] then
			return
		end
		desktop_run(action)
	end
	local buttons: { Node } = {}
	for _, action in DESKTOP_ACTIONS do
		local a = action
		table.insert(buttons, Btn {
			i18n.t(`desktop.{a}`),
			size = "xs",
			tone = "secondary",
			busy = function(): boolean
				return desktop_pending_box:get()
			end,
			on_click = function()
				run(a)
			end,
		})
	end
	return ui.column {
		w_full = true,
		gap = "$space.2",
		ui.text { i18n.t("desktop.title"), text_size = "$font.size.sm", weight = "$font.weight.semibold", color = "$text.secondary" },
		ui.row { wrap = true, gap = "$space.2", table.unpack(buttons) },
		ui.text {
			function(): string
				local r = desktop_result_box:get()
				if r == nil then
					return i18n.s("desktop.never")
				end
				if r.kind == "running" then
					return i18n.s("desktop.running")
				end
				if r.kind == "refused" then
					return i18n.s("desktop.refused", { why = r.err })
				end
				local parts = { i18n.s("desktop.exit", { code = tostring(r.status) }) }
				local out, errt = trim(r.stdout), trim(r.stderr)
				if out ~= "" then
					table.insert(parts, out)
				end
				if errt ~= "" then
					table.insert(parts, errt)
				end
				return table.concat(parts, " · ")
			end,
			text_size = "$font.size.xs",
			color = "$text.secondary",
		},
		ui.text { i18n.t("desktop.hint"), text_size = "$font.size.xs", color = "$text.tertiary" },
	}
end

local function page_content(): Node
	local function worst(): string
		return model.aggregate(zeron.sessions())
	end
	local hero = ui.column {
		items = "center",
		gap = "$space.2",
		w_full = true,
		Pet { size = 160, status = worst },
		ui.text {
			function(): string
				if next(zeron.sessions()) == nil then
					return i18n.s("status.none")
				end
				return i18n.s("status." .. worst())
			end,
			text_size = "$font.size.md",
			weight = "$font.weight.semibold",
			color = status_color(worst),
		},
	}
	return ui.scroll {
		w_full = true,
		h_full = true,
		ui.column {
			w_full = true,
			gap = "$space.5",
			px = "$space.4",
			py = "$space.4",
			hero,
			ui.show(function(): boolean
				return show_usage:get() ~= false
			end, usage_section()),
			chats_section(),
			ui.show(function(): boolean
				return desktop_controls:get() == true
			end, desktop_section()),
		},
	}
end

zeron.page {
	id = "home",
	title = i18n.t("page.home"),
	build = function(_page: any): Node
		-- Rebuilt on language change so usage notes, the picture's label
		-- and computed captions follow; the usage refresher lives above
		-- this and is left alone.
		return ui.presence(function(): string
			return i18n.language()
		end, function(_lang: string): Node
			return page_content()
		end)
	end,
}

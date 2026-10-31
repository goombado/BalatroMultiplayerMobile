-- Steam login for iOS/Android builds (src/steam_bridge.lua). Opened from
-- MultiplayerAPI's account button while the bridge is not logged in
-- (src/provider.lua's open_login). Two ways in:
-- - Account name and password, typed with the phone's own keyboard. Steam then
--   asks for approval in the Steam app (one tap), or for a Steam Guard code.
-- - "Show QR code": scan it with the Steam app on another device.
-- The bridge talks to Steam directly; nothing here keeps a password after it is
-- handed over. Never shown on desktop.

local Bridge = MPM.bridge
local S = Bridge.State
local TextField = MPM.load('src/text_field.lua')

-- Vendored encoder (lib/qrencode). If it fails to load, the overlay still
-- offers the password login.
local qrencode
do
	local ok, mod = pcall(MPM.load, 'lib/qrencode/qrencode.lua')
	if ok and type(mod) == 'table' and mod.qrcode then
		qrencode = mod
	end
end

local QR_SIZE = 3.2
local QR_REFRESH = 0.25
local FIELD_WIDTH = 5.2

local _state = { open = false, url = '', state = nil, last_check = -math.huge, show_qr = false, qr_url = nil, qr_matrix = nil, password_sent = false, notice = nil }

local fields = {
	user = TextField.new({ max = 64 }),
	pass = TextField.new({ secret = true, max = 128 }),
	code = TextField.new({ max = 8, upper = true }),
}
-- What each field shows, for the UI text nodes' ref_table.
local shown = { user = '', pass = '', code = '' }
local focused = nil

-----------------------------
-- QR DRAWABLE
-----------------------------

-- Draws a QR matrix as rectangles; one Moveable instead of hundreds of UI nodes.
local QRCodeDrawable = Moveable:extend()

function QRCodeDrawable:init(matrix, size)
	Moveable.init(self, 0, 0, size, size)
	self.matrix = matrix
	self.states.collide.can = false
end

function QRCodeDrawable:draw()
	if not self.states.visible then
		return
	end
	local n = #self.matrix
	-- Four modules of white border: scanners need the quiet zone.
	local cell = self.VT.w / (n + 8)
	prep_draw(self, 1)
	love.graphics.setColor(1, 1, 1, 1)
	love.graphics.rectangle('fill', 0, 0, self.VT.w, self.VT.h)
	love.graphics.setColor(0, 0, 0, 1)
	for x = 1, n do
		local column = self.matrix[x]
		for y = 1, n do
			if column[y] > 0 then
				love.graphics.rectangle('fill', (x + 3) * cell, (y + 3) * cell, cell, cell)
			end
		end
	end
	love.graphics.pop()
end

-----------------------------
-- TEXT FIELDS
-----------------------------

local function refresh_shown()
	for name, field in pairs(fields) do
		shown[name] = TextField.display(field, focused == name)
	end
end

local function focus(name)
	focused = name
	-- MultiplayerAPI's chat opens on 't' unless a text field has focus.
	MPAPI.text_input_active = name ~= nil
	refresh_shown()
	if love.keyboard and love.keyboard.setTextInput then
		love.keyboard.setTextInput(name ~= nil)
	end
end

local function rebuild()
	if _state.open and G.OVERLAY_MENU then
		MPM.login_overlay:update()
	end
end

local function submit_password()
	if TextField.is_empty(fields.user) or TextField.is_empty(fields.pass) then
		_state.notice = 'k_steam_login_missing_fields'
		rebuild()
		return
	end
	_state.notice = nil
	_state.password_sent = true
	_state.show_qr = false
	Bridge.begin_password_login(TextField.value(fields.user), TextField.value(fields.pass))
	TextField.clear(fields.pass)
	focus(nil)
	rebuild()
end

local function submit_code()
	if TextField.is_empty(fields.code) then
		return
	end
	Bridge.submit_guard_code(TextField.value(fields.code))
	TextField.clear(fields.code)
	focus(nil)
	rebuild()
end

-- Return moves from the account name to the password, then submits.
local function on_return()
	if focused == 'user' then
		focus('pass')
		rebuild()
	elseif focused == 'pass' then
		submit_password()
	elseif focused == 'code' then
		submit_code()
	end
end

-- While a field has focus it takes the keyboard; otherwise input goes on to
-- the game untouched. This mod only loads on iOS/Android.
do
	local love_textinput_ref = love.textinput
	function love.textinput(text)
		if focused then
			TextField.insert(fields[focused], text)
			refresh_shown()
			return
		end
		if love_textinput_ref then
			love_textinput_ref(text)
		end
	end

	local love_keypressed_ref = love.keypressed
	function love.keypressed(key, ...)
		if focused then
			if key == 'backspace' then
				TextField.backspace(fields[focused])
				refresh_shown()
			elseif key == 'return' or key == 'kpenter' then
				on_return()
			elseif key == 'escape' then
				focus(nil)
			end
			return
		end
		if love_keypressed_ref then
			return love_keypressed_ref(key, ...)
		end
	end
end

-----------------------------
-- UI FUNCTIONS
-----------------------------

local function text_row(text, scale, colour)
	return {
		n = G.UIT.R,
		config = { align = 'cm', padding = 0.05 },
		nodes = {
			{ n = G.UIT.T, config = { text = text, scale = scale, colour = colour or G.C.UI.TEXT_LIGHT, shadow = true } },
		},
	}
end

local function button_row(label, button, colour)
	return {
		n = G.UIT.R,
		config = { align = 'cm', padding = 0.05 },
		nodes = {
			{
				n = G.UIT.C,
				config = {
					align = 'cm',
					padding = 0.12,
					minw = FIELD_WIDTH,
					minh = 0.65,
					r = 0.1,
					colour = colour or G.C.BLUE,
					shadow = true,
					hover = true,
					button = button,
				},
				nodes = {
					{ n = G.UIT.T, config = { text = label, scale = 0.42, colour = G.C.UI.TEXT_LIGHT, shadow = true } },
				},
			},
		},
	}
end

-- A tappable box showing the field; tapping it brings up the keyboard.
local function field_row(name, label_key)
	local is_focused = focused == name
	return {
		n = G.UIT.R,
		config = { align = 'cm', padding = 0.04 },
		nodes = {
			{
				n = G.UIT.C,
				config = { align = 'cl', minw = 1.9 },
				nodes = {
					{ n = G.UIT.T, config = { text = localize(label_key), scale = 0.32, colour = G.C.UI.TEXT_LIGHT, shadow = true } },
				},
			},
			{
				n = G.UIT.C,
				config = {
					align = 'cl',
					padding = 0.1,
					minw = FIELD_WIDTH - 1.9,
					maxw = FIELD_WIDTH - 1.9,
					minh = 0.55,
					r = 0.1,
					colour = is_focused and G.C.GREY or G.C.BLACK,
					outline = is_focused and 1 or nil,
					outline_colour = G.C.WHITE,
					hover = true,
					button = 'mpm_steam_focus_' .. name,
				},
				nodes = {
					{ n = G.UIT.T, config = { ref_table = shown, ref_value = name, scale = 0.38, maxw = FIELD_WIDTH - 2.1, colour = G.C.UI.TEXT_LIGHT } },
				},
			},
		},
	}
end

-- Encodes only when Steam rotates the URL, not on every rebuild.
local function qr_matrix(url)
	if _state.qr_url ~= url then
		local ok, matrix = qrencode.qrcode(url)
		_state.qr_url, _state.qr_matrix = url, (ok and type(matrix) == 'table') and matrix or nil
	end
	return _state.qr_matrix
end

local function qr_row(url)
	local matrix = qr_matrix(url)
	if not matrix then
		return nil
	end
	return {
		n = G.UIT.R,
		config = { align = 'cm', padding = 0.1 },
		nodes = {
			{ n = G.UIT.O, config = { object = QRCodeDrawable(matrix, QR_SIZE) } },
		},
	}
end

local function add_login_form(rows)
	rows[#rows + 1] = field_row('user', 'k_steam_login_username')
	rows[#rows + 1] = field_row('pass', 'k_steam_login_password')
	if _state.notice then
		rows[#rows + 1] = text_row(localize(_state.notice), 0.3, G.C.RED)
	end
	rows[#rows + 1] = button_row(localize('b_steam_login'), 'mpm_steam_login_submit', G.C.BLUE)
end

local function add_qr_section(rows, state)
	if not qrencode then
		return
	end
	if _state.show_qr then
		local qr = state == S.AWAITING_QR and _state.url ~= '' and qr_row(_state.url)
		if qr then
			rows[#rows + 1] = qr
			rows[#rows + 1] = text_row(localize('k_steam_login_scan'), 0.32)
		else
			rows[#rows + 1] = text_row(localize('k_steam_login_working'), 0.32, G.C.GOLD)
		end
	end
	local label = localize(_state.show_qr and 'b_steam_login_hide_qr' or 'b_steam_login_show_qr')
	rows[#rows + 1] = button_row(label, 'mpm_steam_login_toggle_qr', G.C.GREY)
end

local create_UIBox_steam_login_overlay = function()
	local state = Bridge.state()
	local rows = { text_row(localize('k_steam_login_title'), 0.55) }
	local show_error = true

	if not Bridge.present() then
		rows[#rows + 1] = text_row(localize('k_offline_steam_bridge_missing'), 0.38, G.C.RED)
		rows[#rows + 1] = text_row(localize('k_steam_login_install_hint'), 0.3)
	elseif state == S.READY then
		local name = Bridge.get_persona_name() or ''
		rows[#rows + 1] = text_row(localize('k_steam_login_ready') .. (name ~= '' and (': ' .. name) or ''), 0.38, G.C.GREEN)
		rows[#rows + 1] = button_row(localize('b_steam_login_logout'), 'mpm_steam_logout', G.C.RED)
		show_error = false
	elseif state == S.NOT_OWNED then
		rows[#rows + 1] = text_row(localize('k_offline_steam_not_owned'), 0.38, G.C.RED)
		rows[#rows + 1] = button_row(localize('b_steam_login_logout'), 'mpm_steam_logout', G.C.RED)
	elseif state == S.AWAITING_GUARD_CODE then
		rows[#rows + 1] = text_row(localize('k_steam_login_approve'), 0.38, G.C.GOLD)
		local by_email = Bridge.guard_kind() == 'email'
		rows[#rows + 1] = text_row(localize(by_email and 'k_steam_login_guard_email' or 'k_steam_login_guard_app'), 0.32)
		rows[#rows + 1] = field_row('code', 'k_steam_login_code')
		rows[#rows + 1] = button_row(localize('b_steam_login_submit_code'), 'mpm_steam_login_submit_code', G.C.BLUE)
		rows[#rows + 1] = button_row(localize('b_steam_login_cancel'), 'mpm_steam_login_cancel', G.C.GREY)
	elseif state == S.AUTHENTICATING and _state.password_sent then
		-- No code offered: Steam waits for approval in the app.
		rows[#rows + 1] = text_row(localize('k_steam_login_approve'), 0.38, G.C.GOLD)
		rows[#rows + 1] = button_row(localize('b_steam_login_cancel'), 'mpm_steam_login_cancel', G.C.GREY)
	elseif state == S.LOGGED_OUT or state == S.ERROR or state == S.AWAITING_QR or (state == S.AUTHENTICATING and _state.show_qr) then
		add_login_form(rows)
		add_qr_section(rows, state)
	else
		rows[#rows + 1] = text_row(localize('k_steam_login_working'), 0.38, G.C.GOLD)
	end

	local err = Bridge.last_error()
	if show_error and err ~= '' then
		for _, line in ipairs(MPAPI.wrap_words(err, 48, 3)) do
			rows[#rows + 1] = text_row(line, 0.28, G.C.RED)
		end
	end

	rows[#rows + 1] = { n = G.UIT.R, config = { minh = 0.15 } }
	rows[#rows + 1] = text_row(localize('k_steam_login_security_1'), 0.26, G.C.UI.TEXT_INACTIVE)
	rows[#rows + 1] = text_row(localize('k_steam_login_security_2'), 0.26, G.C.UI.TEXT_INACTIVE)
	rows[#rows + 1] = button_row(localize('b_steam_login_close'), 'mpm_steam_login_close', G.C.ORANGE)

	local contents = {
		{
			n = G.UIT.C,
			config = { align = 'cm', minw = 8, padding = 0.2, r = 0.1, colour = G.C.CLEAR },
			nodes = rows,
		},
	}
	return create_UIBox_generic_options({ snap_back = true, no_back = true, contents = contents })
end

-----------------------------
-- LOGIC FUNCTIONS
-----------------------------

for name in pairs(fields) do
	G.FUNCS['mpm_steam_focus_' .. name] = function(e)
		focus(name)
		rebuild()
	end
end

G.FUNCS.mpm_steam_login_submit = function(e)
	submit_password()
end

G.FUNCS.mpm_steam_login_submit_code = function(e)
	submit_code()
end

G.FUNCS.mpm_steam_login_cancel = function(e)
	_state.password_sent = false
	focus(nil)
	Bridge.cancel_login()
	rebuild()
end

-- The QR login runs only while its code is on screen.
G.FUNCS.mpm_steam_login_toggle_qr = function(e)
	_state.show_qr = not _state.show_qr
	focus(nil)
	local state = Bridge.state()
	if _state.show_qr and (state == S.LOGGED_OUT or state == S.ERROR) then
		_state.password_sent = false
		Bridge.begin_qr_login()
	elseif not _state.show_qr and state == S.AWAITING_QR then
		Bridge.cancel_login()
	end
	rebuild()
end

-- Also the account overlay's button (src/provider.lua): that overlay closes;
-- this one stays open on the login form.
G.FUNCS.mpm_steam_logout = function(e)
	Bridge.logout()
	MPAPI.disconnect()
	if not _state.open and G.OVERLAY_MENU then
		G.FUNCS.exit_overlay_menu()
	end
end

local function abandon_login()
	local state = Bridge.state()
	if state == S.AWAITING_QR or state == S.AWAITING_GUARD_CODE or state == S.AUTHENTICATING then
		Bridge.cancel_login()
	end
end

G.FUNCS.mpm_steam_login_close = function(e)
	_state.open = false
	focus(nil)
	TextField.clear(fields.pass)
	TextField.clear(fields.code)
	abandon_login()
	G.FUNCS.exit_overlay_menu()
end

-----------------------------
-- GLOBAL UI ELEMENT
-----------------------------

MPM.login_overlay = MPAPI.ui_element(create_UIBox_steam_login_overlay)

function MPM.open_login_overlay()
	_state.open = true
	_state.show_qr = false
	_state.password_sent = false
	_state.notice = nil
	_state.url = Bridge.qr_url()
	_state.state = Bridge.state()
	focus(nil)
	MPM.login_overlay:as_overlay()
end

-- Rebuild while open when the bridge moves on or Steam rotates the QR code;
-- close by itself once logged in (MultiplayerAPI then connects).
local MPAPI_update_ref = MPAPI.update
MPAPI.update = function()
	if _state.open and G.OVERLAY_MENU then
		local now = love.timer.getTime()
		if now - _state.last_check >= QR_REFRESH then
			_state.last_check = now
			local state, url = Bridge.state(), Bridge.qr_url()
			if state == S.READY and _state.state ~= S.READY then
				_state.open = false
				focus(nil)
				G.FUNCS.exit_overlay_menu()
			elseif state ~= _state.state or url ~= _state.url then
				if state == S.LOGGED_OUT or state == S.ERROR then
					_state.password_sent = false
				end
				_state.state, _state.url = state, url
				MPM.login_overlay:update()
			end
		end
	elseif _state.open then
		_state.open = false
		focus(nil)
	end
	MPAPI_update_ref()
end

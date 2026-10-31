--[[
    The mobile Steam bridge (bmp-steam-bridge): a native library that logs
    into Steam itself and mints genuine Balatro auth session tickets, for
    iOS/Android builds that have no Steam client. The server checks those
    tickets exactly as it checks desktop ones.

    iOS: the dylib is injected into the app (LiveContainer tweak / Sideloadly),
    so its symbols are already in the process. Android: native_stage.lua
    copies it out of the mod and we load it by absolute path.

    Main thread only. Nothing here calls back from C: the state is polled
    (provider.lua, from MPAPI.update), because LuaJIT FFI callbacks are
    unreliable without JIT (iOS).
]]

local M = {}

M.ABI_VERSION = 1
M.State = {
	UNINITIALISED = 0,
	LOGGED_OUT = 1,
	AWAITING_QR = 2,
	AWAITING_GUARD_CODE = 3,
	AUTHENTICATING = 4,
	CONNECTING = 5,
	READY = 6,
	NOT_OWNED = 7,
	ERROR = 8,
}
local S = M.State

-- Pure. Has the bridge finished what it does by itself at launch (log in from
-- a stored token)? Until then a connect attempt would only fail.
function M.is_settled(state)
	return state ~= S.UNINITIALISED and state ~= S.CONNECTING and state ~= S.AUTHENTICATING
end

-- Pure. The offline reason for a bridge that is present but not READY.
function M.offline_reason_for(state)
	if state == S.NOT_OWNED then
		return MPAPI.OfflineReason.STEAM_NOT_OWNED
	end
	return MPAPI.OfflineReason.STEAM_LOGIN_REQUIRED
end

-- Pure. How to reach the library: 'process' (iOS, already injected), 'path'
-- (Android, staged), or nil (desktop, or Android without a staged copy).
function M.load_plan(os_name, native_dir)
	if os_name == 'iOS' then
		return 'process'
	elseif os_name == 'Android' and native_dir and native_dir ~= '' then
		return 'path'
	end
	return nil
end

local ffi_ok, ffi = pcall(require, 'ffi')
local lib
local last_state = nil
local listeners = {}
M.problem = nil -- 'missing' | 'version' | 'stage_failed': why present() is false on a phone

local function cdef(decl)
	pcall(ffi.cdef, decl)
end

local function str(p)
	if p == nil then
		return ''
	end
	return ffi.string(p)
end

-- Loads and starts the bridge once. Returns present(). stage_error is
-- native_stage.lua's failure ('failed' or 'refuse') on Android, if any.
function M.load(stage_error)
	if lib or M.problem or not ffi_ok then
		return lib ~= nil
	end
	if stage_error == 'failed' or stage_error == 'refuse' then
		M.problem = 'stage_failed'
		return false
	end
	local how = M.load_plan(MPM.platform.os(), os.getenv('BMP_NATIVE_DIR'))
	if not how then
		M.problem = 'missing'
		return false
	end

	cdef('void* dlsym(void* handle, const char* symbol);')
	cdef([[
		int bmp_bridge_version(void);
		void bmp_bridge_init(const char *data_dir);
		int bmp_bridge_state(void);
		const char *bmp_bridge_last_error(void);
		const char *bmp_bridge_qr_url(void);
		const char *bmp_bridge_guard_kind(void);
		void bmp_bridge_begin_qr_login(void);
		void bmp_bridge_begin_password_login(const char *user, const char *pass);
		void bmp_bridge_submit_guard_code(const char *code);
		void bmp_bridge_cancel_login(void);
		void bmp_bridge_logout(void);
		const char *bmp_bridge_steam_id(void);
		const char *bmp_bridge_persona_name(void);
		void *SteamAPI_SteamUser_v023(void);
		uint32_t SteamAPI_ISteamUser_GetAuthSessionTicket(void *self, void *buf, int max, uint32_t *out_len, const void *identity);
		void SteamAPI_ISteamUser_CancelAuthTicket(void *self, uint32_t handle);
	]])

	local candidate
	if how == 'process' then
		-- Look before touching ffi.C: a missing symbol there is an error, a
		-- NULL from dlsym is just "the tweak isn't installed". RTLD_DEFAULT is
		-- (void*)-2 on Darwin.
		if ffi.C.dlsym(ffi.cast('void*', -2), 'bmp_bridge_version') == nil then
			M.problem = 'missing'
			return false
		end
		candidate = ffi.C
	else
		local ok, loaded = pcall(ffi.load, os.getenv('BMP_NATIVE_DIR') .. '/libbmp_steam.so')
		if not ok then
			MPAPI.sendWarnMessage('[Mobile] Could not load libbmp_steam.so: ' .. tostring(loaded))
			M.problem = 'missing'
			return false
		end
		candidate = loaded
	end

	local ok, version = pcall(function()
		return candidate.bmp_bridge_version()
	end)
	if not ok or version ~= M.ABI_VERSION then
		MPAPI.sendWarnMessage('[Mobile] Unsupported bridge ABI ' .. tostring(version) .. ' (want ' .. M.ABI_VERSION .. '): update the bridge')
		M.problem = 'version'
		return false
	end
	lib = candidate
	lib.bmp_bridge_init(love.filesystem.getSaveDirectory())
	MPAPI.sendDebugMessage('[Mobile] Loaded (' .. how .. ')')
	return true
end

function M.present()
	return lib ~= nil
end

function M.state()
	return lib and lib.bmp_bridge_state() or S.UNINITIALISED
end

function M.ready()
	return M.state() == S.READY
end

function M.settled()
	return M.is_settled(M.state())
end

function M.offline_reason()
	return M.offline_reason_for(M.state())
end

function M.last_error()
	return lib and str(lib.bmp_bridge_last_error()) or ''
end

function M.qr_url()
	return lib and str(lib.bmp_bridge_qr_url()) or ''
end

-- 'app' (Steam Guard code from the authenticator), 'email', or ''.
function M.guard_kind()
	return lib and str(lib.bmp_bridge_guard_kind()) or ''
end

function M.begin_qr_login()
	if lib then
		lib.bmp_bridge_begin_qr_login()
	end
end

-- The bridge copies both strings; the caller should drop its own copies.
function M.begin_password_login(user, pass)
	if lib then
		lib.bmp_bridge_begin_password_login(user, pass)
	end
end

function M.submit_guard_code(code)
	if lib then
		lib.bmp_bridge_submit_guard_code(code)
	end
end

function M.cancel_login()
	if lib then
		lib.bmp_bridge_cancel_login()
	end
end

function M.logout()
	if lib then
		lib.bmp_bridge_logout()
	end
end

-- Same contract as networking/steam.lua's desktop functions.
function M.get_steam_id()
	local id = lib and str(lib.bmp_bridge_steam_id()) or ''
	if id == '' then
		return nil, 'Steam bridge not logged in'
	end
	return id
end

function M.get_persona_name()
	local name = lib and str(lib.bmp_bridge_persona_name()) or ''
	if name == '' then
		return nil, 'Could not get persona name'
	end
	return name
end

function M.get_auth_ticket()
	if not lib then
		return nil, 'Steam bridge not loaded'
	end
	local user = lib.SteamAPI_SteamUser_v023()
	if user == nil then
		return nil, 'Steam bridge not ready'
	end
	local max_ticket = 1024
	local buf = ffi.new('uint8_t[?]', max_ticket)
	local actual_len = ffi.new('uint32_t[1]')
	local handle = lib.SteamAPI_ISteamUser_GetAuthSessionTicket(user, buf, max_ticket, actual_len, nil)
	if handle == 0 then
		return nil, 'GetAuthSessionTicket returned invalid handle'
	end
	local hex_parts = {}
	for i = 0, actual_len[0] - 1 do
		hex_parts[#hex_parts + 1] = string.format('%02x', buf[i])
	end
	return { ticket = table.concat(hex_parts), handle = tonumber(handle) }
end

function M.cancel_auth_ticket(handle)
	if lib and handle then
		lib.SteamAPI_ISteamUser_CancelAuthTicket(nil, handle)
	end
end

-- fn(new_state, old_state), on the main thread from MPAPI.update.
function M.on_state_change(fn)
	listeners[#listeners + 1] = fn
end

local POLL_INTERVAL = 0.25
local last_poll = -math.huge

function M.poll(now)
	if not lib then
		return
	end
	if now - last_poll < POLL_INTERVAL then
		return
	end
	last_poll = now
	local state = lib.bmp_bridge_state()
	if state == last_state then
		return
	end
	local old = last_state
	last_state = state
	for _, fn in ipairs(listeners) do
		local ok, err = pcall(fn, state, old)
		if not ok then
			MPAPI.sendWarnMessage('[Mobile] state listener error: ' .. tostring(err))
		end
	end
end

return M

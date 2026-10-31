-- Contract test for Multiplayer Mobile's pure decisions: platform detection,
-- Android native staging, the bridge's load plan and offline reasons, the
-- login text fields and the vendored QR encoder. Run from mod/:
-- luajit tests/mobile_test.lua
local checks = 0
local function check(condition, message)
	checks = checks + 1
	assert(condition, message)
end

-- The parts of MultiplayerAPI this mod relies on, as main.lua extends them.
MPAPI = {
	OfflineReason = {
		STEAM_NOT_RUNNING = 'steam_not_running',
		SERVER_UNREACHABLE = 'server_unreachable',
		STEAM_BRIDGE_MISSING = 'steam_bridge_missing',
		STEAM_LOGIN_REQUIRED = 'steam_login_required',
		STEAM_NOT_OWNED = 'steam_not_owned',
		NATIVE_STAGE_FAILED = 'native_stage_failed',
	},
}
local R = MPAPI.OfflineReason
MPM = {}

-----------------------------
-- platform.lua
-----------------------------

local platform = dofile('src/platform.lua')
check(platform.is_mobile_os('iOS') and platform.is_mobile_os('Android'), 'iOS and Android are mobile')
check(not platform.is_mobile_os('OS X') and not platform.is_mobile_os('Windows') and not platform.is_mobile_os('Linux') and not platform.is_mobile_os(nil), 'desktop is not mobile')
MPM.platform = platform

-----------------------------
-- native_stage.lua
-----------------------------

local native = dofile('src/native_stage.lua')
local sums = table.concat({
	'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa  libbmp_crypto.so',
	'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb  libbmp_ssl.so',
	'0123456789ABcdef0123456789abcdef0123456789abcdef0123456789abcdef *libbmp_steam.so',
}, '\n') .. '\n'
local parsed = native.parse_sums(sums)
check(parsed['libbmp_ssl.so'] == string.rep('b', 64), 'two-space sums lines parse')
check(parsed['libbmp_steam.so'] == '0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef', 'binary-mode (*) lines parse, hashes lower-cased')
check(native.version_dir(sums) == '0123456789ab', 'version folder is the bridge hash prefix')
check(native.version_dir('') == nil, 'no sums, no version')

local base = { os = 'Android', save_dir = '/data/user/0/com.example.balatro/files/save/game', shipped_sums = sums }
local function plan(over)
	local f = {}
	for k, v in pairs(base) do
		f[k] = v
	end
	for k, v in pairs(over or {}) do
		f[k] = v
	end
	return native.plan(f)
end
check(plan({ os = 'iOS' }) == 'skip' and plan({ os = 'OS X' }) == 'skip' and plan({ os = 'Windows' }) == 'skip', 'only Android stages')
check(plan() == 'stage', 'first launch stages')
check(plan({ staged_sums = sums }) == 'ready', 'an identical staged copy is reused')
check(plan({ staged_sums = 'old' }) == 'stage', 'an update stages again')
check(plan({ shipped_sums = false }) == 'missing', 'a mod without Android libraries says so')
check(plan({ shipped_sums = 'aaaa  libbmp_ssl.so\n' }) == 'missing', 'an incomplete SHA256SUMS counts as missing')
check(plan({ save_dir = '/storage/emulated/0/Android/data/x/files' }) == 'refuse', 'external storage is refused (noexec)')
check(plan({ save_dir = '/data/data/com.example/files' }) == 'stage', '/data/data is app-private')

local writes, modes = {}, {}
local err = native.stage({
	read = function(name)
		return 'bytes:' .. name
	end,
	write = function(rel, bytes)
		writes[#writes + 1] = rel
		return true
	end,
	chmod = function(rel, mode)
		modes[rel] = mode
		return 0
	end,
}, 'MPM_native/0123456789ab', sums)
check(err == nil, 'staging succeeds')
check(writes[#writes] == 'MPM_native/0123456789ab/SHA256SUMS', 'SHA256SUMS is written last, marking a complete copy')
check(modes['MPM_native/0123456789ab/libbmp_steam.so'] == tonumber('555', 8), 'libraries are made 0555')
check(native.stage({
	read = function(name)
		return name ~= 'libbmp_ssl.so' and 'x' or nil
	end,
	write = function()
		return true
	end,
	chmod = function()
		return 0
	end,
}, 'd', sums) ~= nil, 'a missing library fails before anything is written')

-----------------------------
-- steam_bridge.lua (pure parts)
-----------------------------

local bridge_mod = dofile('src/steam_bridge.lua')
local BS = bridge_mod.State
check(not bridge_mod.is_settled(BS.UNINITIALISED) and not bridge_mod.is_settled(BS.CONNECTING) and not bridge_mod.is_settled(BS.AUTHENTICATING), 'still logging in from a stored token')
check(bridge_mod.is_settled(BS.READY) and bridge_mod.is_settled(BS.LOGGED_OUT) and bridge_mod.is_settled(BS.NOT_OWNED) and bridge_mod.is_settled(BS.ERROR), 'done, one way or the other')
check(bridge_mod.offline_reason_for(BS.NOT_OWNED) == R.STEAM_NOT_OWNED, 'not owned has its own reason')
check(bridge_mod.offline_reason_for(BS.LOGGED_OUT) == R.STEAM_LOGIN_REQUIRED, 'logged out asks for a login')
check(bridge_mod.load_plan('iOS', nil) == 'process', 'iOS uses the injected library')
check(bridge_mod.load_plan('Android', '/data/x') == 'path', 'Android loads the staged copy')
check(bridge_mod.load_plan('Android', nil) == nil and bridge_mod.load_plan('Android', '') == nil, 'Android without a staged copy has no bridge')
check(bridge_mod.load_plan('OS X', '/x') == nil and bridge_mod.load_plan('Windows', nil) == nil, 'desktop never loads the bridge')

-----------------------------
-- provider.lua
-----------------------------

MPM.bridge = bridge_mod
local provider = dofile('src/provider.lua')
check(provider.offline_reason(false, 'missing', nil) == R.STEAM_BRIDGE_MISSING, 'no bridge: says it is not installed')
check(provider.offline_reason(false, 'stage_failed', nil) == R.NATIVE_STAGE_FAILED, 'Android staging failed: says so')
check(provider.offline_reason(true, nil, BS.LOGGED_OUT) == R.STEAM_LOGIN_REQUIRED, 'logged out: asks for a login')
check(provider.offline_reason(true, nil, BS.NOT_OWNED) == R.STEAM_NOT_OWNED, 'account without Balatro: says so')
check(provider.provider.present(), 'the provider is always present on a phone, so its reasons reach the menu')

-----------------------------
-- Offline reasons have menu text
-----------------------------

local en = dofile('localization/en-us.lua').misc.dictionary
for _, reason in ipairs({ R.STEAM_BRIDGE_MISSING, R.STEAM_LOGIN_REQUIRED, R.STEAM_NOT_OWNED, R.NATIVE_STAGE_FAILED }) do
	check(type(en['k_offline_' .. reason]) == 'string', 'en-us has text for offline reason ' .. reason)
end
for _, key in ipairs({
	'k_steam_login_title',
	'k_steam_login_scan',
	'k_steam_login_working',
	'k_steam_login_ready',
	'k_steam_login_install_hint',
	'k_steam_login_security_1',
	'k_steam_login_security_2',
	'b_steam_login',
	'b_steam_login_show_qr',
	'k_steam_login_username',
	'k_steam_login_password',
	'k_steam_login_code',
	'k_steam_login_missing_fields',
	'k_steam_login_approve',
	'k_steam_login_guard_app',
	'k_steam_login_guard_email',
	'b_steam_login_submit_code',
	'b_steam_login_cancel',
	'b_steam_login_hide_qr',
	'b_steam_login_logout',
	'b_steam_login_close',
}) do
	check(type(en[key]) == 'string', 'en-us has ' .. key)
end

-----------------------------
-- Vendored QR encoder
-----------------------------

local qrencode = dofile('lib/qrencode/qrencode.lua')
local qr_ok, qr = qrencode.qrcode('https://s.team/q/1/2895736215183928197')
check(qr_ok and #qr == 29 and #qr[1] == 29, 'qrencode encodes a Steam QR login URL (version 3, 29x29)')

-----------------------------
-- Login text fields
-----------------------------

local TextField = dofile('src/text_field.lua')
local pw = TextField.new({ secret = true })
TextField.insert(pw, 'a')
check(TextField.display(pw) == 'a', 'a secret field shows the character just typed')
TextField.insert(pw, 'B')
check(TextField.display(pw) == '*B', 'the previous character turns into a mask when the next is typed')
TextField.backspace(pw)
check(TextField.display(pw) == '*', 'backspace removes a character and leaves only masks')
TextField.insert(pw, 'xyz')
check(TextField.display(pw) == '***z' and TextField.value(pw) == 'axyz', 'a paste reveals only its last character')
check(TextField.display(pw, true) == '***z_', 'a focused field ends in a caret')
TextField.insert(pw, '\n\t')
check(TextField.value(pw) == 'axyz', 'control characters are dropped')
TextField.insert(pw, 'é')
check(TextField.value(pw) == 'axyzé' and TextField.display(pw) == '****é', 'multi-byte characters count as one')
TextField.clear(pw)
check(TextField.is_empty(pw) and TextField.display(pw) == '', 'clear empties the field')
local user = TextField.new({ max = 3 })
TextField.insert(user, 'MyName')
check(TextField.display(user) == 'MyN', 'a plain field shows its text as typed, up to max')
local code = TextField.new({ upper = true })
TextField.insert(code, 'ab3cd')
check(TextField.value(code) == 'AB3CD', 'the Steam Guard code field upper-cases')

print(('ok   %d checks'):format(checks))

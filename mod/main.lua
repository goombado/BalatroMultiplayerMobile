--[[
    Multiplayer Mobile: Steam login for Balatro Multiplayer on iOS/Android.

    A phone has no Steam client, so MultiplayerAPI can't get a Steam auth
    ticket by itself. This mod's native Steam bridge logs in to Steam and mints
    genuine Balatro tickets; it is registered as MultiplayerAPI's Steam
    provider (MPAPI.networking.steam.register_provider). The server checks
    those tickets exactly as it checks desktop ones.

    Does nothing on desktop: the Steam client provides everything there.
]]

MPM = MPM or {}
MPM.id = SMODS.current_mod.id
MPM.path = SMODS.current_mod.path

local loaded = {}
function MPM.load(file)
	if loaded[file] == nil then
		local chunk, err = SMODS.load_file(file, MPM.id)
		if not chunk then
			error('MultiplayerMobile: failed to load ' .. file .. ': ' .. tostring(err), 0)
		end
		loaded[file] = chunk() or true
	end
	return loaded[file]
end

MPM.platform = MPM.load('src/platform.lua')
if not MPM.platform.is_mobile() then
	return
end

local steam = MPAPI and MPAPI.networking and MPAPI.networking.steam
if not (steam and steam.register_provider and MPAPI.add_account_overlay_button) then
	sendWarnMessage('This MultiplayerAPI has no Steam provider support: update MultiplayerAPI', 'MultiplayerMobile')
	return
end

-- Offline reasons this mod gives MultiplayerAPI's menu (text in localization/).
MPAPI.OfflineReason.STEAM_BRIDGE_MISSING = 'steam_bridge_missing'
MPAPI.OfflineReason.STEAM_LOGIN_REQUIRED = 'steam_login_required'
MPAPI.OfflineReason.STEAM_NOT_OWNED = 'steam_not_owned'
MPAPI.OfflineReason.NATIVE_STAGE_FAILED = 'native_stage_failed'

-- Android: copy the libraries out of the mod before anything loads them.
local _, stage_error = MPM.load('src/native_stage.lua').stage_if_needed()

MPM.bridge = MPM.load('src/steam_bridge.lua')
MPM.bridge.load(stage_error)

MPM.load('src/login_overlay.lua')
MPM.load('src/provider.lua').install()

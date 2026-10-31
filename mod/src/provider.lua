--[[
    Registers the Steam bridge with MultiplayerAPI as its Steam provider
    (MPAPI.networking.steam.register_provider): on a phone, MultiplayerAPI
    then gets its Steam id, persona name and auth tickets from the bridge, and
    its menu sends the player to this mod's login. Only loaded on iOS/Android.
]]

local Bridge = MPM.bridge
local S = Bridge.State
local R = MPAPI.OfflineReason

local M = {}

-- Pure. Why MultiplayerAPI can't log in yet: the bridge is missing, its
-- Android libraries failed to stage, or it is not logged in to an account
-- that owns Balatro.
function M.offline_reason(present, problem, state)
	if not present then
		return problem == 'stage_failed' and R.NATIVE_STAGE_FAILED or R.STEAM_BRIDGE_MISSING
	end
	return Bridge.offline_reason_for(state)
end

M.provider = {
	-- Always present on a phone: when the bridge is missing, its reason is
	-- what the menu should show.
	present = function()
		return true
	end,
	ready = function()
		return Bridge.ready()
	end,
	-- A bridge logging in from a stored login is worth waiting for; a missing
	-- one is not.
	settled = function()
		return not Bridge.present() or Bridge.settled()
	end,
	offline_reason = function()
		return M.offline_reason(Bridge.present(), Bridge.problem, Bridge.state())
	end,
	get_steam_id = Bridge.get_steam_id,
	get_persona_name = Bridge.get_persona_name,
	get_auth_ticket = Bridge.get_auth_ticket,
	cancel_auth_ticket = Bridge.cancel_auth_ticket,
	login_needed = function()
		return not Bridge.ready()
	end,
	login_label = function()
		return localize('b_steam_login')
	end,
	open_login = function()
		MPM.open_login_overlay()
	end,
}

function M.install()
	local steam = MPAPI.networking.steam
	steam.register_provider(M.provider)

	-- Poll the bridge every frame; MultiplayerAPI hears about every change.
	local MPAPI_update_ref = MPAPI.update
	MPAPI.update = function()
		Bridge.poll(love.timer.getTime())
		MPAPI_update_ref()
	end
	Bridge.on_state_change(function()
		steam.provider_changed()
	end)

	-- Account overlay: log out of the bridge's Steam account. Only while the
	-- bridge is the provider, so never on desktop.
	MPAPI.add_account_overlay_button(function()
		if steam.provider() ~= M.provider or not Bridge.present() then
			return nil
		end
		return UIBox_button({ label = { localize('b_steam_login_logout') }, button = 'mpm_steam_logout', minh = 0.7, scale = 0.4, colour = G.C.RED, focus_args = { nav = 'wide' } })
	end)
end

return M

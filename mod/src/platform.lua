--[[
    Which platform the game runs on. LuaJIT reports ffi.os 'OSX' on iOS and
    'Linux' on Android, so only love.system.getOS() tells a phone apart.
]]

local M = {}

local MOBILE = { iOS = true, Android = true }

-- Pure.
function M.is_mobile_os(os_name)
	return MOBILE[os_name] == true
end

function M.os()
	if love and love.system and love.system.getOS then
		return love.system.getOS()
	end
	return nil
end

function M.is_mobile()
	return M.is_mobile_os(M.os())
end

return M

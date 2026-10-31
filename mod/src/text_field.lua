-- A text field's contents and what it shows, for the Steam login overlay
-- (ui/steam_login_overlay.lua). Pure: no LÖVE, no UI. Typing arrives from
-- the platform keyboard as UTF-8 text, so the field keeps whole characters.
--
-- A secret field shows '*' for every character except the one just typed,
-- which stays visible until the next one arrives; backspace hides it again.

local M = {}

local MASK = '*'
local CHARACTER = '[%z\1-\127\194-\244][\128-\191]*'

-- spec: { secret = bool, max = number, upper = bool }
function M.new(spec)
	spec = spec or {}
	return { chars = {}, secret = spec.secret == true, max = spec.max or 64, upper = spec.upper == true, reveal = false }
end

-- Appends typed (or pasted) text, dropping control characters and anything
-- past `max`. A paste reveals only its last character.
function M.insert(field, text)
	local added = false
	for char in tostring(text):gmatch(CHARACTER) do
		if #field.chars >= field.max then
			break
		end
		if char:byte(1) >= 32 and char ~= '\127' then
			field.chars[#field.chars + 1] = field.upper and char:upper() or char
			added = true
		end
	end
	if added then
		field.reveal = field.secret
	end
	return added
end

function M.backspace(field)
	field.chars[#field.chars] = nil
	field.reveal = false
end

function M.clear(field)
	field.chars = {}
	field.reveal = false
end

function M.value(field)
	return table.concat(field.chars)
end

function M.is_empty(field)
	return #field.chars == 0
end

-- What the field shows; a focused field ends in a caret.
function M.display(field, focused)
	local n = #field.chars
	local text
	if not field.secret then
		text = table.concat(field.chars)
	elseif field.reveal and n > 0 then
		text = MASK:rep(n - 1) .. field.chars[n]
	else
		text = MASK:rep(n)
	end
	return focused and (text .. '_') or text
end

return M

--[[
    Stages the Android native libraries (the Steam bridge and its OpenSSL)
    out of the mod into app-private storage, where the dynamic linker can load
    them.

    Android can't dlopen from inside a zip-mounted mod, and external storage is
    mounted noexec, so the main thread copies native/android-arm64/* into
    <save dir>/MPM_native/<version>/. It exports BMP_NATIVE_DIR for
    steam_bridge.lua and BET_OPENSSL_SSL_PATH / BET_OPENSSL_CRYPTO_PATH for
    MultiplayerAPI's OpenSSL loader (worker threads included). The version
    folder comes from the shipped SHA256SUMS, so an update never overwrites a
    library the process may already have loaded. The release build puts the
    libraries in native/; the repository holds none. iOS never touches it.
]]

local STAGE_DIR_NAME = 'MPM_native'
local SOURCE_DIR = 'native/android-arm64'
local SUMS_FILE = 'SHA256SUMS'
local VERSION_FILE = 'libbmp_steam.so'
local LIBRARY_FILES = { 'libbmp_crypto.so', 'libbmp_ssl.so', 'libbmp_steam.so' }

local M = {}

M.LIBRARY_FILES = LIBRARY_FILES

-- Pure. "<sha256>  <file>" lines -> { [file] = sha256 }.
function M.parse_sums(text)
	local sums = {}
	for hash, file in (text or ''):gmatch('(%x+)%s+%*?([^\r\n]+)') do
		sums[file] = hash:lower()
	end
	return sums
end

-- Pure. The staged folder's name: the first 12 hex digits of the bridge's hash.
function M.version_dir(sums_text)
	local hash = M.parse_sums(sums_text)[VERSION_FILE]
	return hash and hash:sub(1, 12) or nil
end

-- Pure. facts: os (love.system.getOS()), save_dir, shipped_sums (the mod's
-- SHA256SUMS text or nil), staged_sums (the staged copy's, or nil).
--   'skip'    - not Android
--   'missing' - the mod ships no Android libraries
--   'refuse'  - the save dir is not app-private internal storage (noexec)
--   'ready'   - an identical copy is already staged
--   'stage'   - copy now
function M.plan(facts)
	if facts.os ~= 'Android' then
		return 'skip'
	end
	local sums = M.parse_sums(facts.shipped_sums)
	for _, file in ipairs(LIBRARY_FILES) do
		if not sums[file] then
			return 'missing'
		end
	end
	if type(facts.save_dir) ~= 'string' or not facts.save_dir:match('^/data/') then
		return 'refuse'
	end
	if facts.staged_sums == facts.shipped_sums then
		return 'ready'
	end
	return 'stage'
end

-- deps: read(file_name) -> bytes|nil (from the mod's native/android-arm64/),
-- write(relative_path, bytes) -> boolean, chmod(absolute_path, mode) -> 0 on
-- success. SHA256SUMS is written last: it marks a complete copy.
-- Returns nil or a message.
function M.stage(deps, version_rel_dir, sums_text)
	local contents = {}
	for _, file_name in ipairs(LIBRARY_FILES) do
		local bytes = deps.read(file_name)
		if not bytes then
			return 'bundled ' .. file_name .. ' not found in the mod'
		end
		contents[file_name] = bytes
	end
	for _, file_name in ipairs(LIBRARY_FILES) do
		local rel = version_rel_dir .. '/' .. file_name
		if not deps.write(rel, contents[file_name]) then
			return 'could not write ' .. file_name .. ' to the save directory'
		end
		-- 0555: readable and executable, never rewritten in place.
		if deps.chmod(rel, tonumber('555', 8)) ~= 0 then
			return 'could not chmod ' .. file_name
		end
	end
	if not deps.write(version_rel_dir .. '/' .. SUMS_FILE, sums_text) then
		return 'could not write ' .. SUMS_FILE
	end
end

-- The effectful wiring. Returns (absolute dir) or (nil, offline-reason
-- message); on Android a failure leaves BMP_NATIVE_DIR unset, so OpenSSL and
-- the bridge report themselves missing.
function M.stage_if_needed()
	if not (love and love.system and love.system.getOS() == 'Android') then
		return nil
	end
	local ffi = require('ffi')

	-- Steamodded mounts a zip under a path that may be relative to Mods/.
	local mount_roots = { (MPM.path:gsub('/*$', '')) }
	local relative = MPM.path:match('[/\\]Mods[/\\](.+)$')
	if relative then
		mount_roots[#mount_roots + 1] = (relative:gsub('\\', '/'):gsub('/*$', ''))
	end
	local function read(file_name)
		-- An unpacked mod folder: love.filesystem can't read absolute paths, NFS can.
		local direct = NFS and NFS.read(mount_roots[1] .. '/' .. SOURCE_DIR .. '/' .. file_name)
		if direct then
			return direct
		end
		for _, root in ipairs(mount_roots) do
			local bytes = love.filesystem.read(root .. '/' .. SOURCE_DIR .. '/' .. file_name)
			if bytes then
				return bytes
			end
		end
	end

	local save_dir = love.filesystem.getSaveDirectory():gsub('/*$', '')
	local shipped = read(SUMS_FILE)
	local version = M.version_dir(shipped)
	local rel_dir = version and (STAGE_DIR_NAME .. '/' .. version) or nil
	local plan = M.plan({
		os = 'Android',
		save_dir = save_dir,
		shipped_sums = shipped,
		staged_sums = rel_dir and love.filesystem.read(rel_dir .. '/' .. SUMS_FILE) or nil,
	})
	if plan == 'missing' then
		MPAPI.sendWarnMessage('[Mobile] This copy of the mod has no Android libraries (native/android-arm64): install a release build')
		return nil, 'missing'
	elseif plan == 'refuse' then
		MPAPI.sendWarnMessage('[Mobile] The save directory is not in app-private storage, so native libraries cannot load: ' .. save_dir)
		return nil, 'refuse'
	end

	pcall(ffi.cdef, 'int setenv(const char *name, const char *value, int overwrite);')
	pcall(ffi.cdef, 'int chmod(const char *path, unsigned int mode);')
	if plan == 'stage' then
		love.filesystem.createDirectory(rel_dir)
		local err = M.stage({
			read = read,
			write = love.filesystem.write,
			chmod = function(rel, mode)
				return ffi.C.chmod(save_dir .. '/' .. rel, mode)
			end,
		}, rel_dir, shipped)
		if err then
			MPAPI.sendWarnMessage('[Mobile] Could not stage the Android libraries: ' .. err)
			return nil, 'failed'
		end
		MPAPI.sendDebugMessage('[Mobile] Staged the Android libraries to ' .. save_dir .. '/' .. rel_dir)
	end
	local dir = save_dir .. '/' .. rel_dir
	ffi.C.setenv('BMP_NATIVE_DIR', dir, 1)
	ffi.C.setenv('BET_OPENSSL_SSL_PATH', dir .. '/libbmp_ssl.so', 1)
	ffi.C.setenv('BET_OPENSSL_CRYPTO_PATH', dir .. '/libbmp_crypto.so', 1)
	return dir
end

return M

-- CoopClientMod: starts the sync client as soon as the client's game is up.
--
-- Why this exists instead of an external launcher: every attempt to start coop-link on the other
-- machine from outside needed a live remote-execution channel, and that channel kept dying (an SMB
-- share cannot spawn a process, and a wedged agent took the session down three times). The game
-- already loads Lua at startup. Starting the sync client from inside the game removes the whole
-- class of problem: if the game is running, the client is running.
--
-- This mod does not read or write game memory. It only spawns coop-link.exe, which does the work.

local ROLE_PATH = "Mods/CoopClientMod/Scripts/coop_client.txt"
local LOG_PATH  = "Mods/CoopClientMod/Scripts/coop_client.log"
-- No committed default: the host address differs per network, and a hardcoded LAN IP would point a
-- stranger's install at someone else's machine. With coop_client.txt absent or empty the client does
-- not start, and says so in the log.
local DEFAULT_SUPERVISOR = ""

local function log(fmt, ...)
    local ok, msg = pcall(string.format, fmt, ...)
    if not ok then msg = tostring(fmt) end
    print("[CoopClient] " .. msg .. "\n")
    local f = io.open(LOG_PATH, "a")
    if f then f:write(os.date("%H:%M:%S ") .. msg .. "\n"); f:close() end
end

-- One line, `<host>:<port>` or `off`. Keeping it a file means a different host needs no rebuild.
local function supervisor()
    local f = io.open(ROLE_PATH, "r")
    if not f then return DEFAULT_SUPERVISOR end
    local line = f:read("*l")
    f:close()
    if not line then return DEFAULT_SUPERVISOR end
    line = line:gsub("%s+", "")
    if line == "" then return DEFAULT_SUPERVISOR end
    return line
end

local started = false

local function startClient()
    if started then return end
    local target = supervisor()
    if target == "off" then
        log("disabled by %s", ROLE_PATH)
        started = true
        return
    end
    if target == "" then
        -- Not fatal: the game runs fine single-player. Starting coop-link with an empty target would
        -- launch it against nothing and log a confusing failure instead.
        log("no host address in %s - co-op client not started", ROLE_PATH)
        started = true
        return
    end
    started = true

    -- `start` returns immediately, so this does not block the game thread. Running coop-link
    -- directly through os.execute would block it for the client's entire lifetime.
    local cmd = string.format(
        'start "" /min coop-link.exe --role client --supervisor %s --hz 20', target)
    log("starting: %s", cmd)
    local ok, why, code = os.execute(cmd)
    log("os.execute ok=%s why=%s code=%s", tostring(ok), tostring(why), tostring(code))
end

-- Starting before the world exists means coop-link auto-selects a process that is still the
-- launcher. Waiting for the first game mode is the same signal the other mods use.
NotifyOnNewObject("/Script/Engine.GameModeBase", function()
    ExecuteWithDelay(5000, function()
        local g = supervisor()
        log("game mode seen, supervisor=%s", g)
        startClient()
    end)
end)

-- Also fire on a plain timer, because a mod loaded mid-session never sees the object being created.
ExecuteWithDelay(20000, function()
    log("timer tick, supervisor=%s", supervisor())
    startClient()
end)

log("CoopClientMod loaded, supervisor=%s", supervisor())

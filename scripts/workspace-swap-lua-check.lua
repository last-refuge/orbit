-- Execute the production chunk against stateful workspace objects, including
-- refusals at each mutation. Run: lua scripts/workspace-swap-lua-check.lua
local file = assert(io.open('crates/orbit-hyprland/src/swap.lua'))
local chunk = file:read('*a'); file:close()
chunk = chunk:gsub('@SOURCE@', '1'):gsub('@TARGET@', '2')
    :gsub('@SOURCE_WINDOWS@', "'address:0xa','address:0xb'")
    :gsub('@TARGET_WINDOWS@', "'address:0xc'")
local function scenario(fail, stale, missing)
    local a, b = {id=1, windows=2, tree={ratio=.37}}, {id=2, windows=1, tree={ratio=.61}}
    local occupied = {id=4294967294, windows=0}
    local windows = {['address:0xa']={workspace=a}, ['address:0xb']={workspace=a}, ['address:0xc']={workspace=b}}
    local mutations = 0
    local function get(id)
        for _, w in ipairs({a,b,occupied}) do if w.id == tonumber(id) then return w end end
    end
    hl = {
        get_workspace=get, get_window=function(address) return windows[address] end,
        dispatch=function(f) f() end,
        dsp={workspace={change_id=function(args) return function()
            mutations = mutations + 1
            if mutations == fail then return end -- real dispatch may report an error without throwing
            assert(not get(args.id))
            get(args.workspace).id = args.id
        end end}},
    }
    if stale then windows['address:0xa'].workspace=b end
    if missing then hl.dsp.workspace.change_id=nil end
    local ok, err = pcall(assert(load(chunk)))
    assert(occupied.id == 4294967294)
    assert(a.tree.ratio == .37 and b.tree.ratio == .61)
    if fail or stale or missing then
        assert(not ok, tostring(err))
        assert(a.id == 1 and b.id == 2, 'rollback failed')
        if stale or missing then assert(mutations == 0) end
    else
        assert(ok, err)
        assert(a.id == 2 and b.id == 1)
        assert(windows['address:0xa'].workspace == a)
        assert(windows['address:0xc'].workspace == b)
    end
end
scenario()
for step=1,3 do scenario(step) end
scenario(nil,true)
scenario(nil,false,true)
print('PASS production Lua: swap, occupied scratch ID, all three rollback points, stale contents, unsupported compositor')

-- Orbit workspace swap: preserve the workspace object and its layout tree.
local source, target = @SOURCE@, @TARGET@
assert(type(hl.dsp.workspace.change_id) == 'function', 'Workspace swapping requires Hyprland change_id support')
local a, b = hl.get_workspace(tostring(source)), hl.get_workspace(tostring(target))
assert(a and b and a.id == source and b.id == target, 'A workspace disappeared; try again')
local function check(ws, addresses)
    assert(ws.windows == #addresses, 'Workspace contents changed; try again')
    for _, address in ipairs(addresses) do
        local w = hl.get_window(address)
        assert(w and w.workspace and w.workspace.id == ws.id, 'Workspace contents changed; try again')
    end
end
check(a, {@SOURCE_WINDOWS@})
check(b, {@TARGET_WINDOWS@})
local temporary = 4294967294
while hl.get_workspace(tostring(temporary)) do temporary = temporary - 1 end
assert(temporary > 0, 'No free workspace number')
local function change(ws, id)
    hl.dispatch(hl.dsp.workspace.change_id({workspace = tostring(ws.id), id = id}))
    assert(ws.id == id, 'Hyprland refused a workspace ID change')
end
local ok, reason = pcall(function()
    change(a, temporary)
    change(b, source)
    change(a, target)
end)
if not ok then
    -- Undo completed steps in reverse order; never move individual windows.
    local restored = pcall(function()
        if a.id == target then change(a, temporary) end
        if b.id == source then change(b, target) end
        if a.id == temporary then change(a, source) end
        assert(a.id == source and b.id == target)
    end)
    if not restored then error('Workspace swap failed and rollback was incomplete; refresh before retrying') end
    error('Workspace swap cancelled; original workspaces restored: ' .. tostring(reason))
end
assert(a.id == target and b.id == source, 'Workspace swap could not be verified')

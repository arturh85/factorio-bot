-- What does a reserved beacon lane COST in siting?
--
-- The owner wants space left for beacons from the beginning so production can
-- be improved later without a teardown -- smelters already force one and that
-- is what he is trying to avoid repeating.
--
-- **The beacon's supply area does not reach us.** The prototype arrives with
-- collision_box, collision_mask, entity_type, mine_result, mining_time and name
-- and nothing else, so the row spacing a beacon actually needs is unknown here.
-- Picking a number from memory is the hard-coded-rate defect in another hat, so
-- this measures the COST as a function of the gap instead: whatever the real
-- spacing turns out to be, the price of it is already known.
--
-- A reserved lane is not empty space in a blueprint -- blueprints hold only
-- entities. It is the machines moving apart, which grows the footprint, and a
-- bigger footprint can refuse siting on ground that works today. An inserter
-- reaches exactly one tile and cannot span the lane, so each extra tile also
-- needs another belt tile to carry ore across it.
--
-- Planning only. Nothing is built: `goal.plan` is pure, so all seven variants
-- cost one game and no placements.
print("start beacon lane cost")

local VARIANTS = {
  { gap = 0, w = 10, h = 13, bp = "0eNqd1dFugyAUgOF34Vobj4KKL7CHWJZF27ONRNEALmsa3320vWiz0uScXSr1kxR+PIlhXHFxxgbRnYQJOInu7l4mxn7AMd576ZciXqINJhj0ons9XS+O73adBnSig0zYfsL44+B665fZhTw+fFaW2cfHZnt+yY/o6p3KxFF0xU5tmTgYh/vraLtlD2zJZoHCVmy2pLCSzVYUVrFZSWFrNqsobMNmawrbstmGwmo221JYKNiuJrn8zoAUGvyjNFJqwG8NSLEBvTZ4duTIlEvPrWS59N4qlksPTrJcenGK5dKT089cKFPfCnpzLQ+mR9fw4Ft0xs0233+hT23f+/2bYm6JDauz6HJjPboQxx6tgrNWpfwrT8Ya+5kfnBnHRBAXOgfCh13xZEmXa/q/0Tw709PLdSvNh9li/hH9fo+JjXtVk4vV8qf3eNKm56d58wPYtrdMfKPzlyFVl1pqrSpZQhXfsP0CkdVS/A==" },
  { gap = 1, w = 10, h = 13, bp = "0eNqd1d1ugyAUB/B34VobDx8qvsAeYlkWbdlGomiALmsa3320vWgzaXLOLhH5mYPnD2c2jEezeOsi687MRjOx7uFZwcZ+MGN69tIvkIbGRRutCax7Pd8Gp3d3nAbjWQcFc/1k0svR9y4ss49lWnxRljmkZbO7fOSHdfVOFezEumqn1oIdrDf722y7FhuWk1nAsILMcgwryazAsIrMSgxbk1mFYRsyW2PYlsw2GFaT2RbDQkV2Ncql5wxQQYN/JA0VNaBnDVBhA3za4NmRI3MuPm6c5OLzJkguPnCS5OITp0guPnL6mQs8d1fgM9fSYHzoGhp8D531syv3Xybk2vexf3PMPWLD0TvjS+uC8THNba2K8q+4/CtP1ln3WR68HcdMIK50CYiLXdFkiZdrchu0qL1o8LvcPIPzbXBPWoizM+VH8vu9yf29G5vtAk2ue3uC5woXFb3wrZytXACxcoB1fSvYt/HhOqdqrqXWSkgOIn1i/QU5Io02" },
  { gap = 2, w = 10, h = 13, bp = "0eNqd1UtugzAQgOG7eA1Rxg9eF+ghqqqCZNpaAoOMqRpF3L1OskhUHGmmSyB8CDN/fBZdv+DkrQuiOQsbcBDNw7lM9G2HfTz30k4yHqILNlicRfN6vh2c3t0ydOhFA5lw7YDxx8G3bp5GH/J480WZxjneNrrLQ35EU+xMJk6i2e/Mmomj9Xi4Xa3WbMNKNgsUVrFZSWE1m1UU1rBZTWELNmsobMlmCwpbsdmSwtZstqKwsGe7Ncnldwak0OAfpZFSA35rQIoN6LXBs78cnXLpuUmWS+9NsVx6cJrl0oszLJeeXP3MBZnaK+jNVTyYHl3Jg+/RWT+6/PCFc2p8H+c3xdwT6xbv0OfWzehDvLa19pxvJfVfebDOus/86G3fJ4K40jkQNnbDkzVdLthjUJHWomTPLc2t6F+vfAanx+te2hxGh/lH9NsDpibsxqamS/Gz2u4MqRdXwF5QIiz5K7qVk0uqFHNJAdb1LRPf6OfrNVPIWte1UVqCio9YfwGBrcco" },
  { gap = 3, w = 10, h = 13, bp = "0eNqd1e1ugyAUBuB74bc2PXyoeAO9iGVZtGUbiaIBuqxpvPfRNlmbSZNz9lOQB/WcV86sH45m9tZF1p6ZjWZk7cNYwYauN0Ma23WzSJfGRRutCax9Od8uTm/uOPbGsxYK5rrRpJuj71yYJx/LtPiizFNIyyZ32eSbtdVGFezE2u1GLQU7WG/2t9lmKVYsJ7OAYQWZ5RhWklmBYRWZlRi2IrMKw9ZktsKwDZmtMawmsw2GhS3Z1SiXnjNABQ3+kTRU1ICeNUCFDfBpg2e/HJlz8XHjJBefN0Fy8YGTJBefOEVy8ZHTz1zgubMCn7mGBuNDV9Pge+isn1y5/zQh176P/Ztj7hHrj94ZX1oXjI9pbm1tKbXi8q88WmfdR3nwdhgygbjSJSAOdkWTJV6uyG3QoL5FTe5bnIvP2W/xcLDGt0X9DM72rbgnLcTJmfI9+d3eZJ741hFNrm0FkAu1PnJyLy44uVJIWNBLhZQlvVZrOV8sRSwWwLK8FuzL+HCdUxXXUmslJAeRtlh+AGH0AX0=" },
  { gap = 4, w = 11, h = 13, bp = "0eNqd1e1ugjAYhuFz6W8w9IuvE9hBLMsC2m1NsJBSlxnDua9qombU5Hn3EyoXKO9tT6wfDmby1gXWnpgNZs/ah3MZG7reDPHcSzepeGhcsMGambWvp+vB8d0d9r3xrOUZc93exA8H37l5Gn3I48VnZRrneNnozjf5YW250Rk7srbY6CVjO+vN9rpaL9mKFWSWI6wkswJhFZmVCKvJrELYksxqhK3IbImwNZmtELYhszXC8oLsNpBL74xDofF/lAalxumtcSg2jtfGn/3lqJSL5yZILt6bJLl4cIrk4sVpkosn1zxzuUjtFXhzNQ3Go6to8D0660eXb7/MnBrfx/lNMffE+oN3xufWzcaHuLa2Csq7EuqvvLfOus985+0wJIK40DkHNnZNkxUul+QxqKHfoiLPLebind1eHgbjod0mDIJlgc9b9QxOBiHvpc1hdCb/iH63NYknllc21YMU5AlY72XJLy7JIwDCij4DoKzpQwDKJX0K1nJ6DCriGHC+LG8Z+zZ+vqzpUjSqabRUgst4i+UXF6U7yQ==" },
  { gap = 5, w = 12, h = 13, bp = "0eNqd1etugjAYxvF76Wcw9sTpBnYRy7KAdlsTKKTUZcZw76uaqBk1ed59VOTngecvJ9b1BzN56wJrTswGM7Dm4bmM9W1n+vjcSzvp+NC4YIM1M2teT9cHx3d3GDrjWcMz5trBxBcH37p5Gn3I48lnZRrneNrozm/yw5piozN2ZM12o5eM7a03u+vRaslWrCCzHGElmRUIq8isRFhNZhXCFmRWI2xJZguErchsibA1ma0Qlm/Jbg259M44FBr/R2lQapzeGodi43ht/Nlfjkq5eG6C5OK9SZKLB6dILl6cJrl4cvUzl4vUvQJvrqLBeHQlDb5HZ/3o8t2XmVPzfdxvirkn1h28Mz63bjY+xGNra0u5VkL9lQfrrPvM9972fSKIC51z4MauabLC5YI8gwr6LUrybjEX7+x28TAYD+22MAiWeGhckGCOD7l8BidLk/fS5jA6k39Ev92ZxCe+Lq1KhSYleVrrm2TyiyvytkBY08cFygV9XaBc0ucFyhV9X2s5PbCaODDOl+UtY9/Gz5djuhC1qmstleAyvsXyC6dKdhs=" },
  { gap = 6, w = 13, h = 13, bp = "0eNqd1dtugkAUheF3mWsw7DlweoE+RNM0oNN2EhzIgE2N4d2L2mhTxmTtXiryeWD9chJtd7BDcH4S9Um4ye5F/eu5RHRNa7vluadmyJeH1k9ucnYU9fPp+uD46g/71gZRUyJ8s7fLi6fQ+HHow5QuJ5+VoR+X03p/fpMvUecbk4ijqLONmROxc8Fur0fLOVmxks0Swio2KxFWs1mFsIbNaoTN2axB2ILN5ghbstkCYSs2WyIsZWy3glx+ZwSFRv8oDUqN+K0RFBvhtdGjvxwdc/HcJMvFe1MsFw9Os1y8OMNy8eSqRy7J2L0Cb67kwXh0BQ++R+dC79Pthx1j8/293xhzT6w9BG9D6vxow7QcW1sZ51pJ/VfeO+/8e7oLrusiQVzolIAbu+HJGpdz9gxK6Lco2LvFXLyz28XDYDy028IgWOGhkWTBeGikWLDECykewdGE1b29ceq9Td8Wv9nayCf+YWMFK83e7PruG/3ihj1aEM75qwXlgj9bUC75uwXlij9cTNYZf7lrOTpdTczpEs3zSyI+bRgvx0wuK11VRmlJanmL+RsMrrBh" },
}

local ok_count, refused = 0, 0
for _, v in ipairs(VARIANTS) do
  local ok, err = pcall(function() return goal.plan(goal.built(v.bp)) end)
  if ok then
    local plan = goal.plan(goal.built(v.bp))
    local places = 0
    for _, st in ipairs(plan.steps) do
      if st.kind == "place" then places = places + 1 end
    end
    -- Where did it choose? A block pushed off the ore is as much a failure as
    -- one refused outright, and it would not show up as a refusal.
    local minx, miny = 1e9, 1e9
    for _, st in ipairs(plan.steps) do
      if st.kind == "place" and st.pos then
        if st.pos.x < minx then minx = st.pos.x end
        if st.pos.y < miny then miny = st.pos.y end
      end
    end
    print(string.format("  gap=%d  %2dx%-2d  SITED at (%.0f,%.0f), %d placements",
      v.gap, v.w, v.h, minx, miny, places))
    ok_count = ok_count + 1
  else
    local msg = tostring(err):gsub("%s+", " ")
    print(string.format("  gap=%d  %2dx%-2d  REFUSED: %s", v.gap, v.w, v.h, msg:sub(1, 110)))
    refused = refused + 1
  end
end

print("")
print(string.format("%d of %d gap widths site on this map; %d refused",
  ok_count, #VARIANTS, refused))
print("A gap that sites costs nothing but ground. A gap that refuses is the")
print("owner's call, not ours -- the block stops being buildable where it is.")
print("end beacon lane cost")

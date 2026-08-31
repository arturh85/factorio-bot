-- Falsification table for the smelt-chain change.
-- Five rows plus the headline goals, all planned over the whole roster.
print("start falsify")

local kinds = { "walk", "mine", "craft", "place", "insert", "remove", "research" }

local function row(name, g)
  local ok, p = pcall(function() return goal.plan(g) end)
  if not ok then
    print(("ROW %-22s FAIL %s"):format(name, tostring(p)))
    return
  end
  local parts = {}
  for _, k in ipairs(kinds) do
    local n = p:count { kind = k }
    if n > 0 then parts[#parts + 1] = k .. "=" .. n end
  end
  print(("ROW %-22s steps=%d makespan=%d bots=%d %s")
        :format(name, #p.steps, p.makespan, #p.bots, table.concat(parts, " ")))
end

row("mining", goal.have("iron-ore", 30))
row("smelting", goal.have("iron-plate", 30))
row("trigger-research", goal.researched("electronics"))
row("crafting", goal.have("iron-gear-wheel", 10))
row("lab", goal.have("lab", 1))
row("automation", goal.researched("automation"))
row("smelt-big", goal.have("iron-plate", 800))
row("copper-cable", goal.have("copper-cable", 200))

print("end falsify")

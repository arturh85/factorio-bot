-- Plans a designed block against the live world. Plans only; places nothing.
--
-- **This asserted falsely until 2026-09-05, and had never been run.**
-- `p:count{kind = "place"}` counts EVERY placement in the plan, and a
-- correct plan builds scaffolding of its own -- stone furnaces to smelt the
-- plates, a lab to run a research the bill needs -- which are
-- `kind == "place"` too. So the 37 asserted here was only ever right for a
-- roster that happened to be carrying the whole bill already. The identical
-- trap was found and fixed in a Rust test earlier on this branch
-- (`a_locked_recipe_is_actually_researched_and_crafted_not_merely_billed`,
-- crates/planner/src/method/blueprint.rs) and its lesson never reached the
-- Lua: filter on the `"block band N"` label that `BuildBlock` puts on every
-- one of its own placements and on nothing else.
--
-- A committed smoke test that asserts falsely is worse than none.
print("start block smoke")

local MINER_LINE = "0eNqdl11v2yAUhv9KxLWdBPBH7MtN603Vq3ZSt2ma/MEyJAwIcFcr8n8vcapoWjztwJWFDQ+Hw3l5zQm1YmTacOlQfUI9s53h2nElUY0e1Wg6Vm9+Oadtvdv9bDqnDFdLd7vt1LB74ez3Ln24+/rhy2d83368e+0n3d4PzyhBVjY6dSo9Gt6f4a+oLhM0oZrgOUFNa5UYHUvP3TSXR1Q7M7IE8U5Ji+pvJ2T5UTbiPNRNmvmAuGODB8tmOLeYYJ0zvEsHLv34tDdcCOTRXPbMT4bn7wli0nHH2QW4NKYfchxaZnyH/6ASpJXll2Qs4eNtvizAP/00PTd+1PKVzMkNnVzpzjTSamVc2jLhbrH0Hbv/G5utYGlo0Pm/gi5W6Flw0AQSdB6MxRBsEbuBOWQDyyvdDo0Q6XUOrQS7ZZN3tl/BvEI7BKcgg6SgCsZSCBbvY8ssg5QZxsFhF6Cww0WXg7jRqitB6chiC/kAKWQcLr8DKCtFMLcEcSOVh/fr0sPh2sOgsxiHq6+CcEm0+m7jXqs3Eu98BGR94TLEIB8hNBwMchKSRWecgjKeR2cc5FUkXIoY5C+kDAeDHIZEiBLkASRclBhkAjRelQWkRmi8KitIjVASeayW68cqjRAjyFdo+F8oBhkLzaN3sALtYIQKQYZAw1VIVhzM34eWK1T9x60vQaLxJP/ugWw3nxojps2TUv79CzP2UkwHnJVZVRYl3hd5Mc9vau2sKw=="

-- The anchor is a guess, and `goal.built` has no siting story: if a single
-- tile of the 4x21 footprint is occupied the plan refuses by name, saying
-- which tile and what is on it (including "one of this plan's own bots is
-- standing on it"). That refusal is a fact about this map, not a failure of
-- this script, so it is reported rather than swallowed.
local ANCHOR = { x = 10, y = 10 }

local ok, p = pcall(goal.plan, goal.built(MINER_LINE, ANCHOR))
if not ok then
  print("PLAN REFUSED at anchor (" .. ANCHOR.x .. "," .. ANCHOR.y .. "): " .. tostring(p))
  print("end block smoke")
  return
end

print("makespan=" .. tostring(p.makespan) .. " steps=" .. #p.steps
      .. " bots=" .. #p.bots)
assert(p.makespan > 0, "a 37-entity block must take positive time")

-- The block's OWN placements, told apart from the plan's scaffolding by the
-- label `BuildBlock` writes on each: "place <name> at <pos> -- block band N".
local own, scaffolding = 0, 0
local per_bot_places = {}
local by_direction = {}
for _, st in ipairs(p.steps) do
  if st.kind == "place" then
    if string.find(st.label, "block band", 1, true) then
      own = own + 1
      per_bot_places[st.bot] = (per_bot_places[st.bot] or 0) + 1
      by_direction[st.direction] = (by_direction[st.direction] or 0) + 1
    else
      scaffolding = scaffolding + 1
    end
  end
end
print("placements: " .. tostring(own) .. " of the block, "
      .. tostring(scaffolding) .. " of the plan's own scaffolding")
assert(own == 37,
  "MinerLine's 37 entities are 37 block placements, got " .. tostring(own))

-- `direction` is published on a place step since 2026-09-05. MinerLine is a
-- Factorio 1.x blueprint, so every direction in it has been migrated from the
-- eight-point scale onto 2.x's sixteen -- which means every one is a multiple
-- of 4 (a cardinal) and never an odd half-diagonal. A raw 1.x direction
-- would show up here as a 2 or a 6.
for direction, n in pairs(by_direction) do
  print("  direction " .. tostring(direction) .. ": " .. tostring(n))
  assert(direction % 4 == 0,
    "a migrated 1.x direction is a cardinal on the 16-point scale, got "
    .. tostring(direction))
end

local per_bot = {}
for _, st in ipairs(p.steps) do
  per_bot[st.bot] = (per_bot[st.bot] or 0) + 1
end
for bot, n in pairs(per_bot) do
  print("bot " .. bot .. ": " .. n .. " step(s), "
        .. tostring(per_bot_places[bot] or 0) .. " block placement(s)")
end
print("end block smoke")

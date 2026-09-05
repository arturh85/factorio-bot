-- Plans a designed block against the live world. Plans only; places nothing.
print("start block smoke")

local MINER_LINE = "0eNqdl11v2yAUhv9KxLWdBPBH7MtN603Vq3ZSt2ma/MEyJAwIcFcr8n8vcapoWjztwJWFDQ+Hw3l5zQm1YmTacOlQfUI9s53h2nElUY0e1Wg6Vm9+Oadtvdv9bDqnDFdLd7vt1LB74ez3Ln24+/rhy2d83368e+0n3d4PzyhBVjY6dSo9Gt6f4a+oLhM0oZrgOUFNa5UYHUvP3TSXR1Q7M7IE8U5Ji+pvJ2T5UTbiPNRNmvmAuGODB8tmOLeYYJ0zvEsHLv34tDdcCOTRXPbMT4bn7wli0nHH2QW4NKYfchxaZnyH/6ASpJXll2Qs4eNtvizAP/00PTd+1PKVzMkNnVzpzjTSamVc2jLhbrH0Hbv/G5utYGlo0Pm/gi5W6Flw0AQSdB6MxRBsEbuBOWQDyyvdDo0Q6XUOrQS7ZZN3tl/BvEI7BKcgg6SgCsZSCBbvY8ssg5QZxsFhF6Cww0WXg7jRqitB6chiC/kAKWQcLr8DKCtFMLcEcSOVh/fr0sPh2sOgsxiHq6+CcEm0+m7jXqs3Eu98BGR94TLEIB8hNBwMchKSRWecgjKeR2cc5FUkXIoY5C+kDAeDHIZEiBLkASRclBhkAjRelQWkRmi8KitIjVASeayW68cqjRAjyFdo+F8oBhkLzaN3sALtYIQKQYZAw1VIVhzM34eWK1T9x60vQaLxJP/ugWw3nxojps2TUv79CzP2UkwHnJVZVRYl3hd5Mc9vau2sKw=="

local p = goal.plan(goal.built(MINER_LINE, {x = 10, y = 10}))
print("makespan=" .. tostring(p.makespan) .. " steps=" .. #p.steps
      .. " bots=" .. #p.bots)
assert(p.makespan > 0, "a 37-entity block must take positive time")

local placed = p:count { kind = "place" }
print("placements: " .. tostring(placed))
assert(placed == 37, "every entity in the block is a placement, got " .. tostring(placed))

local per_bot = {}
for _, st in ipairs(p.steps) do
  per_bot[st.bot] = (per_bot[st.bot] or 0) + 1
end
for bot, n in pairs(per_bot) do print("bot " .. bot .. ": " .. n .. " step(s)") end
print("end block smoke")

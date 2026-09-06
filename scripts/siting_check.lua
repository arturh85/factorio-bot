-- Task 7 (block siting): plan both rcontest.lua fixtures with NO anchor --
-- `goal.built(BP)` sites itself -- and report what it chose.
--
-- Written for the live-run half of Task 7 (see
-- .superpowers/sdd/2026-09-05-block-siting/task-7-brief.md and
-- task-7-offline-report.md, the offline half this script's own numbers must
-- match). Plans only; places nothing; safe against any world, live or fresh.
--
-- **Offline evidence already found MinerLine cannot be sited on this map.**
-- The offline run against the seed-31337 dump exhausted the self-siting
-- search (SEARCH_RADIUS=48, and 64/96/128 checked by hand) with NO valid
-- anchor: MinerLine's transport-belt and small-electric-poles run in a
-- corridor directly BETWEEN its two drill columns, over the same ore the
-- drills stand on, and the planner refuses to place anything but a drill on
-- an ore tile (`Occupant::Resource`, `crates/planner/src/state.rs`). Iron
-- ore on this map is one solid 38x36 blob, so that corridor is ore
-- everywhere the drills are. This is expected to refuse here too, by name
-- (`PlannerError::NoSiteFound`) -- report it as a refusal, not a crash, and
-- do not treat a non-zero exit from `goal.plan` as a script bug.
print("start siting check")

local FURNACE_LINE = blueprints and blueprints.FurnaceLine
local MINER_LINE = blueprints and blueprints.MinerLine
if not FURNACE_LINE or not MINER_LINE then
  -- Not `include`d from rcontest.lua on purpose (see block_run.lua's note):
  -- that file fires rcon.cheat_* calls at its own top level, which this
  -- script does not want. Copied verbatim instead.
  FURNACE_LINE = "0eNqdm81vo0gQxf+ViLOd0NUf0D7ubTXSXuYwh9VoRJweL7sYEODsRlH+98WxZmKNaXiPk5UPfn5Vpqq7X+HX5LE6hbYr6yHZvSZPod93ZTuUTZ3sks/NqduHu7+Goe13Dw/fi/3QdGXz/t/9/b45PjyX4d+H7aemOv1h/37+cvg9DYeXfz59+5Jskr4u2u3QbA9d+XRm/5fsxG+Sl2Sn1NsmKR778bIhbM//15b1IdkN3SlsknLf1H2y+/M16ctDXVTna4eXNoyCyiEcR3JdHM8/DV1R923TDdvHUA3JyCzrpzC+zYhfvLgfmjpsv5+6utiHq2sFuHbftG3otm1VDNeXauDSsmvqmwvN29dNEuqhHMpwifz9h5dv9en4GLoxoFjMm6Rt+vLycb1nWN3b9xSn93bkP5Vd2F/++h7XL1ihsePr2wRIwyDN6DMwVhis/fg06j50w/i7G6CdD9jByiyjLIOxhsHmQMBuPmD/UTjHoqq2oRrfsCv327apwi0tm6epFA40YwJVeKE4iitABv1CzHiReEobXiU5xUXKRKULQeOFoqjWpfBS+akRAyPFovRC2GS5KJnnCV4viuqwQqwsVI8VpGKUWQgbLxlF9VnBa0ZRnVagolnojkIUDdUehSgaqj8KVDT5Qths0Sw0XE0UDdVxNVE0VMvVSNHI0l4MLxqhWq4mtmNUy9VI0chCi9R40QjVIjVeNEK1SI0UjSxsRDVZNLLQcg1eNEK1XIMXjVAt10BFs7C9NUTRUJ3RGPqMJRGJlibpCMlFTsG3h5cLZhKSoZBsBpKjED8D8SjkfDyIUWwKU/QMRcGUmdxagSkzybUapsxk1xqUInPZtTBlLrvwjStz2YXvXJnL7tWt21blMNl2fiyDiCnh6fK2vzYgM+VUpDTXRDwPBUQs0xG7KZ7QrtFNxJNcTfg7sVgNbfBYZDlwlrBiYtocbXJg2jLalMG4+TrvKBa+J7yYCCNLac8ECjVTtMeDcYXxYmJBa94ywdQZ3uTBwJbxYmJhO97iwNRlvCmDgfOV7lEsA57xYiKQPOUtEyjaXPEmDwYWxouJha15ywRTZ3iTBwNbxouJhe14iwNTl/GmDAbOV7pHsQx4xouJQHzKWyZQtF7xJg8GFsaLiYWtecsEU2d4kwcDW8aLiYXteIsDU5fxpgwGzle6R7EMeMaLMbHhYMqbMVC4KiXKJuPI+GFGco78UUyn+il0h64ZXyPsH03E3aA3P59HqNvT+WGJiXfivaoMOYKqlLeuHAZ2RG5kOTfNaYgmJ6NPqy52f+fEARVMhOcsvXxS2NVYfvl0iglTirMJI8Lklz5VFcc2fnKMJV5p4ugIxmc4BzMSHzVrB5U50haNSMuYMxgoLSe91og0D90VShZuC0mZ8xEWorBG8HSIQp1hQGmadJcj0gxzzgClWdKyjkhz2I3hl26MjDkDgCHmpJ8eCdEz+3RMmk5Jk35a2tV4HNhLg9KEdP4j0jSzPwWlGXKcEJHG75E8ps/R4BwDZ+Th4Ur2JI/ZFoGxe2JDgyGvBuSUmx2L2ihiTwJKpNxikKmZbQDINCt9zmguLbOQgyIdswKDzIxZOkHmWv8rmkzKAMNE2pRZtUAmtdyATFnpi8SSaamFBxTJH9cV+OSvpc+6KNnRj/Sj5Iye2KLknH4qHyV7euIKkompvyPJ/IgTJQs9lEXJmh9QomjDD1VRtOXniyh6xUwURWf8OA9F5/wIEkV7fhoHoolnDRRZi8TjBoosxkz4ASCKXvGAMope8Ygyirb86AlFO35chqJXTI5QdM5Pu1C05wdAU+ivm8u3L3dXX3LdJFUxosbf6fu734q+3N99Po7o8xdQN8lz6PrLxfm4Szc+c5lKnXVvb/8DoD9BzA=="
  MINER_LINE = "0eNqdl11v2yAUhv9KxLWdBPBH7MtN603Vq3ZSt2ma/MEyJAwIcFcr8n8vcapoWjztwJWFDQ+Hw3l5zQm1YmTacOlQfUI9s53h2nElUY0e1Wg6Vm9+Oadtvdv9bDqnDFdLd7vt1LB74ez3Ln24+/rhy2d83368e+0n3d4PzyhBVjY6dSo9Gt6f4a+oLhM0oZrgOUFNa5UYHUvP3TSXR1Q7M7IE8U5Ji+pvJ2T5UTbiPNRNmvmAuGODB8tmOLeYYJ0zvEsHLv34tDdcCOTRXPbMT4bn7wli0nHH2QW4NKYfchxaZnyH/6ASpJXll2Qs4eNtvizAP/00PTd+1PKVzMkNnVzpzjTSamVc2jLhbrH0Hbv/G5utYGlo0Pm/gi5W6Flw0AQSdB6MxRBsEbuBOWQDyyvdDo0Q6XUOrQS7ZZN3tl/BvEI7BKcgg6SgCsZSCBbvY8ssg5QZxsFhF6Cww0WXg7jRqitB6chiC/kAKWQcLr8DKCtFMLcEcSOVh/fr0sPh2sOgsxiHq6+CcEm0+m7jXqs3Eu98BGR94TLEIB8hNBwMchKSRWecgjKeR2cc5FUkXIoY5C+kDAeDHIZEiBLkASRclBhkAjRelQWkRmi8KitIjVASeayW68cqjRAjyFdo+F8oBhkLzaN3sALtYIQKQYZAw1VIVhzM34eWK1T9x60vQaLxJP/ugWw3nxojps2TUv79CzP2UkwHnJVZVRYl3hd5Mc9vau2sKw=="
end

-- Every place step this plan owns as ITS OWN block placement, told apart
-- from the plan's own scaffolding by the "block band N" label `BuildBlock`
-- writes (see block_smoke.lua). The FIRST such placement's world position
-- minus that entity's known blueprint offset is one honest reading of the
-- anchor `resolve_site` chose -- and if siting is behaving, every own
-- placement gives the SAME answer, which is checked below rather than
-- assumed.
local function own_placements(p)
  local out = {}
  for _, st in ipairs(p.steps) do
    if st.kind == "place" and string.find(st.label, "block band", 1, true) then
      out[#out + 1] = st
    end
  end
  return out
end

local function check(name, bp)
  print("--- " .. name .. " ---")
  local ok, p = pcall(goal.plan, goal.built(bp))
  if not ok then
    print(name .. ": PLAN REFUSED: " .. tostring(p))
    return
  end
  local placements = own_placements(p)
  print(string.format(
    "%s: makespan=%s steps=%d bots=%d own_placements=%d",
    name, tostring(p.makespan), #p.steps, #p.bots, #placements))
  if #placements > 0 then
    local first = placements[1]
    print(string.format("%s: first own placement %s at (%.1f, %.1f) dir=%s -- %s",
      name, first.entity, first.pos.x, first.pos.y, tostring(first.direction), first.label))
  end
end

check("FurnaceLine", FURNACE_LINE)
check("MinerLine", MINER_LINE)

print("end siting check")

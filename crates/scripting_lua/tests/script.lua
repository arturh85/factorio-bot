-- globals:
-- all_bots: Vec<u32>
-- world: LuaFactorioWorld
-- rcon: LuaFactorioRcon
-- plan: LuaPlanner


blueprints = {
    StarterSteamEngineBoiler = "0eNqdkdEKwjAMRf8lz504nRv0V0Rkm0ECbVrWThxj/242RQXrgz6VhHtPLr0jNKZH3xFH0CNQ6ziA3o8Q6My1mXdx8AgaKKIFBVzbeQoRa5shn4kRJgXEJ7yCzqeDAuRIkfDOWYbhyL1tsBNBmqDAuyAmx/NFAWUiHOQppkl9QDYviK2NydBgGztqM+9MgvVAlSnU9rc8eYpR/BMnSdo9SY0jI5tvOYrVLuUvn35P/utpsUpLS5/6rX4FF+zCIq6qbZ5X1brcyP/fAHsdtKc=",
    StarterScience = "0eNqdlNtuwjAMht/F1w0iPUJfg8tpmtJglWg5VEnYhlDffUlBCKa2jF66tb/f9q/4DI08YmeF9lCfwWnWEW9Ia8U+xj9QlwmcoKa0T0Bwox3UbyFPtJrJmOFPHUINwqOCBDRTMWLOoWqk0C1RjB+ERkIhAvQeAzKwniIka+4q0v49AdReeIGXDobg9KGPqkEbkPPaCXTGhWKjr1ORvFwVw2TZqgg6Frm4dGGNJi0yS74PiBJiq3+00pe1ssVa2U1LaIfWh28j/PzKLwJ/BJL/C1LMQ4obxCkmJUGJ3FvBSWckzg09wSsXO1Y9bpEdvVEsZhLHBWqOpGP8c2yb1WLnlmtuXnJwO76s7UsOTkDoeqmFU0D68FxnLKN0gpA+I2QPhHAFhjNR352tJJaG1xOm8iyuZndxJPz4QusGWLqheZVvq7Ki67Io+/4XVduccQ==",
    FurnaceLine = "0eNqdm81vo0gQxf+ViLOd0NUf0D7ubTXSXuYwh9VoRJweL7sYEODsRlH+98WxZmKNaXiPk5UPfn5Vpqq7X+HX5LE6hbYr6yHZvSZPod93ZTuUTZ3sks/NqduHu7+Goe13Dw/fi/3QdGXz/t/9/b45PjyX4d+H7aemOv1h/37+cvg9DYeXfz59+5Jskr4u2u3QbA9d+XRm/5fsxG+Sl2Sn1NsmKR778bIhbM//15b1IdkN3SlsknLf1H2y+/M16ctDXVTna4eXNoyCyiEcR3JdHM8/DV1R923TDdvHUA3JyCzrpzC+zYhfvLgfmjpsv5+6utiHq2sFuHbftG3otm1VDNeXauDSsmvqmwvN29dNEuqhHMpwifz9h5dv9en4GLoxoFjMm6Rt+vLycb1nWN3b9xSn93bkP5Vd2F/++h7XL1ihsePr2wRIwyDN6DMwVhis/fg06j50w/i7G6CdD9jByiyjLIOxhsHmQMBuPmD/UTjHoqq2oRrfsCv327apwi0tm6epFA40YwJVeKE4iitABv1CzHiReEobXiU5xUXKRKULQeOFoqjWpfBS+akRAyPFovRC2GS5KJnnCV4viuqwQqwsVI8VpGKUWQgbLxlF9VnBa0ZRnVagolnojkIUDdUehSgaqj8KVDT5Qths0Sw0XE0UDdVxNVE0VMvVSNHI0l4MLxqhWq4mtmNUy9VI0chCi9R40QjVIjVeNEK1SI0UjSxsRDVZNLLQcg1eNEK1XIMXjVAt10BFs7C9NUTRUJ3RGPqMJRGJlibpCMlFTsG3h5cLZhKSoZBsBpKjED8D8SjkfDyIUWwKU/QMRcGUmdxagSkzybUapsxk1xqUInPZtTBlLrvwjStz2YXvXJnL7tWt21blMNl2fiyDiCnh6fK2vzYgM+VUpDTXRDwPBUQs0xG7KZ7QrtFNxJNcTfg7sVgNbfBYZDlwlrBiYtocbXJg2jLalMG4+TrvKBa+J7yYCCNLac8ECjVTtMeDcYXxYmJBa94ywdQZ3uTBwJbxYmJhO97iwNRlvCmDgfOV7lEsA57xYiKQPOUtEyjaXPEmDwYWxouJha15ywRTZ3iTBwNbxouJhe14iwNTl/GmDAbOV7pHsQx4xouJQHzKWyZQtF7xJg8GFsaLiYWtecsEU2d4kwcDW8aLiYXteIsDU5fxpgwGzle6R7EMeMaLMbHhYMqbMVC4KiXKJuPI+GFGco78UUyn+il0h64ZXyPsH03E3aA3P59HqNvT+WGJiXfivaoMOYKqlLeuHAZ2RG5kOTfNaYgmJ6NPqy52f+fEARVMhOcsvXxS2NVYfvl0iglTirMJI8Lklz5VFcc2fnKMJV5p4ugIxmc4BzMSHzVrB5U50haNSMuYMxgoLSe91og0D90VShZuC0mZ8xEWorBG8HSIQp1hQGmadJcj0gxzzgClWdKyjkhz2I3hl26MjDkDgCHmpJ8eCdEz+3RMmk5Jk35a2tV4HNhLg9KEdP4j0jSzPwWlGXKcEJHG75E8ps/R4BwDZ+Th4Ur2JI/ZFoGxe2JDgyGvBuSUmx2L2ihiTwJKpNxikKmZbQDINCt9zmguLbOQgyIdswKDzIxZOkHmWv8rmkzKAMNE2pRZtUAmtdyATFnpi8SSaamFBxTJH9cV+OSvpc+6KNnRj/Sj5Iye2KLknH4qHyV7euIKkompvyPJ/IgTJQs9lEXJmh9QomjDD1VRtOXniyh6xUwURWf8OA9F5/wIEkV7fhoHoolnDRRZi8TjBoosxkz4ASCKXvGAMope8Ygyirb86AlFO35chqJXTI5QdM5Pu1C05wdAU+ivm8u3L3dXX3LdJFUxosbf6fu734q+3N99Po7o8xdQN8lz6PrLxfm4Szc+c5lKnXVvb/8DoD9BzA==",
    MinerLine = "0eNqdl11v2yAUhv9KxLWdBPBH7MtN603Vq3ZSt2ma/MEyJAwIcFcr8n8vcapoWjztwJWFDQ+Hw3l5zQm1YmTacOlQfUI9s53h2nElUY0e1Wg6Vm9+Oadtvdv9bDqnDFdLd7vt1LB74ez3Ln24+/rhy2d83368e+0n3d4PzyhBVjY6dSo9Gt6f4a+oLhM0oZrgOUFNa5UYHUvP3TSXR1Q7M7IE8U5Ji+pvJ2T5UTbiPNRNmvmAuGODB8tmOLeYYJ0zvEsHLv34tDdcCOTRXPbMT4bn7wli0nHH2QW4NKYfchxaZnyH/6ASpJXll2Qs4eNtvizAP/00PTd+1PKVzMkNnVzpzjTSamVc2jLhbrH0Hbv/G5utYGlo0Pm/gi5W6Flw0AQSdB6MxRBsEbuBOWQDyyvdDo0Q6XUOrQS7ZZN3tl/BvEI7BKcgg6SgCsZSCBbvY8ssg5QZxsFhF6Cww0WXg7jRqitB6chiC/kAKWQcLr8DKCtFMLcEcSOVh/fr0sPh2sOgsxiHq6+CcEm0+m7jXqs3Eu98BGR94TLEIB8hNBwMchKSRWecgjKeR2cc5FUkXIoY5C+kDAeDHIZEiBLkASRclBhkAjRelQWkRmi8KitIjVASeayW68cqjRAjyFdo+F8oBhkLzaN3sALtYIQKQYZAw1VIVhzM34eWK1T9x60vQaLxJP/ugWw3nxojps2TUv79CzP2UkwHnJVZVRYl3hd5Mc9vau2sKw=="
}




function buildStarterMinerFurnace(bot, ore, count)
    local rect = world.findFreeResourceRect(ore, 2*count, 2, {x=0,y=0})
    dump(rect.leftTop, "leftTop")
    dump(rect.rightBottom, "rightBottom")
    plan.groupStart("Build Starter Miner/Furnace")

    local anchor = rect.leftTop
    for idx=1,count do
        plan.place(bot, {x=anchor.x + ((idx - 1) * 2), y=anchor.y}, "burner-mining-drill")
        plan.place(bot, {x=anchor.x + ((idx - 1) * 2), y=anchor.y-2}, "stone-furnace")
    end

    plan.groupEnd()

    --    local anchor = rect.topLeft

    --    if count > #bots then
    --        for idx, bot in pairs(bots) do
    --            plan.place(bot, {x=anchor.x + ((idx - 1) * 2), y=anchor.y}, "burner-mining-drill")
    --        end
    --
    --    else
    --
    --    end

end



-- Milestone 1: Research Automation
-- Sub Target 1: Mine Iron & Copper
-- Sub Target 2: Build Power Production near Water
-- Sub Target 3: Build Science Facilities



function dump(tbl, label)
    print("dumping " .. label)
    for k,v in pairs(tbl) do
        print("- " .. tostring(k) .. ": " .. tostring(v))
    end
    print("------------")
end


--local recipe = world.recipe("inserter")
--print("recipe: " .. tostring(recipe))
--for k,v in pairs(recipe) do
--    print(k .. ": " .. tostring(v))
--end
--
--dumpPlayers()
function mine_with_bots(bots, search_center, name, type)
    local entities = rcon.findEntitiesInRadius(search_center, 300, name, type)
    local label = name or type
    if #entities < #bots then
        error("not enough " .. label .. " in radius 300")
    end
    plan.groupStart("Mine " .. label .. " with " .. tostring(#bots) .. " Bots")
    for idx,playerId in pairs(bots) do
        plan.mine(playerId, entities[idx].position, entities[idx].name, 1)
    end
    plan.groupEnd()
end


function build_starter_base(bots)
    buildStarterMinerFurnace(bots, "iron-ore", 2)
    --    build_starter_miner_loop("coal", 2)
    --    buildStarterMinerFurnace("iron-ore", 2)
    --    buildStarterMinerFurnace("copper-ore", 2)
    --    build_starter_miner_chest("stone", 2)
    --    build_starter_steam_engine()
    --    build_starter_science()
end

--print("start script")
--mine_with_bots(all_bots, {x=0,y=0}, "rock-huge", nil)
--mine_with_bots(all_bots, {x=0,y=0}, nil, "tree")
--build_starter_base(all_bots)
--print("end script")
--mine_with_bots(bots, {0,0}, nil, "tree")
--build_starter_base()
--research("automation")
--...
--start_rocket()


--function tableLength(t)
--    local count = 0
--    for _ in pairs(t) do count = count + 1 end
--    return count
--end


if all_bot_count == 1 then
    -- place burner-mining-drill & stone-furnace @ iron-ore field
    buildStarterMinerFurnace(all_bots[1], "iron-ore", 1)
    -- loop get min 2x coal from rocks or coal ore -> place in iron burner-mining-drill & stone-furnace -> get all iron-plate until enough for second burner-mining-drill

    -- if not already get enough stone from rocks or stone ore for second burner-mining-drill
    -- craft second burner-mining-drill
    -- place stone burner-mining-drill
    -- loop get min 2x coal -> place in iron burner-mining-drill & stone-furnace -> get all iron-plate until enough for third burner-mining-drill
    -- loop get min 1x coal -> place in stone burner-mining-drill -> get all stone until enough for 2 more burner-mining-drill
    -- craft 2 more burner-mining-drill
    -- place coal burner-mining-drill loop with 2 elements & insert wood

    -- (build burner-mining-drill & stone-furnace @ copper-ore field)
elseif all_bot_count == 2 then
    print("not implemented for 2 bots")
    -- first bot =>
    -- place burner-mining-drill & stone-furnace @ iron-ore field
    -- loop get min 2x coal from rocks or coal ore -> place in iron burner-mining-drill & stone-furnace -> get all iron-plate until enough for second burner-mining-drill
    -- second bot =>
    -- place burner-mining-drill @ stone field
    -- loop get min 1x coal from rocks or coal ore -> place in stone burner-mining-drill
elseif all_bot_count == 3 then
    print("not implemented for 3 bots")
    -- first bot =>
    -- place burner-mining-drill & stone-furnace @ iron-ore field
    -- loop get min 2x coal from rocks or coal ore -> place in iron burner-mining-drill & stone-furnace -> get all iron-plate until enough for second burner-mining-drill
    -- second bot =>
    -- give burner-mining-drill to third bot
    -- third bot =>
    -- wait for second bot to give BMD
    -- place coal burner-mining-drill loop with 2 elements & insert wood
else
    print("not implemented for " .. str(all_bot_count) .. " bots")
end
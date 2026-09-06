-- Task 7 (block siting), LIVE half: goal.built(FurnaceLine) with NO anchor --
-- siting chooses where the block goes -- run for real against a headless
-- 4-bot server, then read every one of the 179 entities back off the live
-- surface and check name + position + direction (+ half, for the two
-- underground belts) against what the plan actually intended.
--
-- Modelled on scripts/furnace_run.lua (Task 6, fixed anchor (60,-100),
-- 178/179 stood correct) with the anchor removed: `goal.built(FURNACE_LINE)`
-- with no second argument is `Site::Anywhere`, so `resolve_site` searches
-- rings outward from a seed (the world origin here, since FurnaceLine has no
-- mining drill) and refuses (`PlannerError::NoSiteFound` /
-- `BlockGroundOccupied`) before placing anything if no ring is clear.
--
-- CHEATED, DISCLOSED: every bot is given the full material set (87
-- transport-belt, 48 inserter, 24 stone-furnace, 13 small-electric-pole, 3
-- small-lamp, 2 splitter, 2 underground-belt) via rcon.cheat_item before
-- planning -- gathering is out of scope for this sub-project, same as every
-- earlier task. No entity is ever cheated in: every placement below is a
-- real goal.run dispatch against a real siting decision.
--
-- THIS SCRIPT DOES NOT TEST WHETHER ANYTHING MOVES. FurnaceLine carries 13
-- small-electric-pole and NO GENERATOR at all -- no boiler, no steam engine,
-- no solar panel -- and nothing here cheats one in. So the 87 belts, 48
-- inserters, 2 splitters and 2 underground belts are checked ONLY for
-- STANDING correctly (position + direction + half, read back off the
-- surface). A furnace smelting when hand-fed was already shown in Task 6 at
-- a fixed anchor; this script does not repeat that test, because the thing
-- under test here is siting, not production.
print("start siting furnace live")

local FURNACE_LINE = "0eNqdm81vo0gQxf+ViLOd0NUf0D7ubTXSXuYwh9VoRJweL7sYEODsRlH+98WxZmKNaXiPk5UPfn5Vpqq7X+HX5LE6hbYr6yHZvSZPod93ZTuUTZ3sks/NqduHu7+Goe13Dw/fi/3QdGXz/t/9/b45PjyX4d+H7aemOv1h/37+cvg9DYeXfz59+5Jskr4u2u3QbA9d+XRm/5fsxG+Sl2Sn1NsmKR778bIhbM//15b1IdkN3SlsknLf1H2y+/M16ctDXVTna4eXNoyCyiEcR3JdHM8/DV1R923TDdvHUA3JyCzrpzC+zYhfvLgfmjpsv5+6utiHq2sFuHbftG3otm1VDNeXauDSsmvqmwvN29dNEuqhHMpwifz9h5dv9en4GLoxoFjMm6Rt+vLycb1nWN3b9xSn93bkP5Vd2F/++h7XL1ihsePr2wRIwyDN6DMwVhis/fg06j50w/i7G6CdD9jByiyjLIOxhsHmQMBuPmD/UTjHoqq2oRrfsCv327apwi0tm6epFA40YwJVeKE4iitABv1CzHiReEobXiU5xUXKRKULQeOFoqjWpfBS+akRAyPFovRC2GS5KJnnCV4viuqwQqwsVI8VpGKUWQgbLxlF9VnBa0ZRnVagolnojkIUDdUehSgaqj8KVDT5Qths0Sw0XE0UDdVxNVE0VMvVSNHI0l4MLxqhWq4mtmNUy9VI0chCi9R40QjVIjVeNEK1SI0UjSxsRDVZNLLQcg1eNEK1XIMXjVAt10BFs7C9NUTRUJ3RGPqMJRGJlibpCMlFTsG3h5cLZhKSoZBsBpKjED8D8SjkfDyIUWwKU/QMRcGUmdxagSkzybUapsxk1xqUInPZtTBlLrvwjStz2YXvXJnL7tWt21blMNl2fiyDiCnh6fK2vzYgM+VUpDTXRDwPBUQs0xG7KZ7QrtFNxJNcTfg7sVgNbfBYZDlwlrBiYtocbXJg2jLalMG4+TrvKBa+J7yYCCNLac8ECjVTtMeDcYXxYmJBa94ywdQZ3uTBwJbxYmJhO97iwNRlvCmDgfOV7lEsA57xYiKQPOUtEyjaXPEmDwYWxouJha15ywRTZ3iTBwNbxouJhe14iwNTl/GmDAbOV7pHsQx4xouJQHzKWyZQtF7xJg8GFsaLiYWtecsEU2d4kwcDW8aLiYXteIsDU5fxpgwGzle6R7EMeMaLMbHhYMqbMVC4KiXKJuPI+GFGco78UUyn+il0h64ZXyPsH03E3aA3P59HqNvT+WGJiXfivaoMOYKqlLeuHAZ2RG5kOTfNaYgmJ6NPqy52f+fEARVMhOcsvXxS2NVYfvl0iglTirMJI8Lklz5VFcc2fnKMJV5p4ugIxmc4BzMSHzVrB5U50haNSMuYMxgoLSe91og0D90VShZuC0mZ8xEWorBG8HSIQp1hQGmadJcj0gxzzgClWdKyjkhz2I3hl26MjDkDgCHmpJ8eCdEz+3RMmk5Jk35a2tV4HNhLg9KEdP4j0jSzPwWlGXKcEJHG75E8ps/R4BwDZ+Th4Ur2JI/ZFoGxe2JDgyGvBuSUmx2L2ihiTwJKpNxikKmZbQDINCt9zmguLbOQgyIdswKDzIxZOkHmWv8rmkzKAMNE2pRZtUAmtdyATFnpi8SSaamFBxTJH9cV+OSvpc+6KNnRj/Sj5Iye2KLknH4qHyV7euIKkompvyPJ/IgTJQs9lEXJmh9QomjDD1VRtOXniyh6xUwURWf8OA9F5/wIEkV7fhoHoolnDRRZi8TjBoosxkz4ASCKXvGAMope8Ygyirb86AlFO35chqJXTI5QdM5Pu1C05wdAU+ivm8u3L3dXX3LdJFUxosbf6fu734q+3N99Po7o8xdQN8lz6PrLxfm4Szc+c5lKnXVvb/8DoD9BzA=="

-- Same 179-entity table Task 6 embedded, hand-verified against the live
-- surface there (178/179 stood at exactly these blueprint-relative
-- coordinates + migrated 16-point direction). Kept exactly as-is: it is the
-- blueprint's own geometry, independent of wherever THIS run's siting puts
-- the anchor. Field names renamed x/y -> rx/ry here only to make clear these
-- are relative-to-anchor, not absolute, since this run has no fixed anchor.
local EXPECTED_REL = {
  { name = "transport-belt", rx = 1.5, ry = 0.5, dir = 4, half = nil },
  { name = "transport-belt", rx = 1.5, ry = 1.5, dir = 0, half = nil },
  { name = "transport-belt", rx = 3.5, ry = 0.5, dir = 4, half = nil },
  { name = "transport-belt", rx = 2.5, ry = 0.5, dir = 4, half = nil },
  { name = "inserter", rx = 5.5, ry = 1.5, dir = 0, half = nil },
  { name = "transport-belt", rx = 5.5, ry = 0.5, dir = 4, half = nil },
  { name = "transport-belt", rx = 4.5, ry = 0.5, dir = 4, half = nil },
  { name = "inserter", rx = 6.5, ry = 1.5, dir = 0, half = nil },
  { name = "small-electric-pole", rx = 7.5, ry = 1.5, dir = 0, half = nil },
  { name = "transport-belt", rx = 7.5, ry = 0.5, dir = 4, half = nil },
  { name = "transport-belt", rx = 6.5, ry = 0.5, dir = 4, half = nil },
  { name = "inserter", rx = 9.5, ry = 1.5, dir = 0, half = nil },
  { name = "transport-belt", rx = 9.5, ry = 0.5, dir = 4, half = nil },
  { name = "transport-belt", rx = 8.5, ry = 0.5, dir = 4, half = nil },
  { name = "inserter", rx = 10.5, ry = 1.5, dir = 0, half = nil },
  { name = "transport-belt", rx = 11.5, ry = 0.5, dir = 4, half = nil },
  { name = "transport-belt", rx = 10.5, ry = 0.5, dir = 4, half = nil },
  { name = "inserter", rx = 13.5, ry = 1.5, dir = 0, half = nil },
  { name = "small-electric-pole", rx = 12.5, ry = 1.5, dir = 0, half = nil },
  { name = "transport-belt", rx = 13.5, ry = 0.5, dir = 4, half = nil },
  { name = "transport-belt", rx = 12.5, ry = 0.5, dir = 4, half = nil },
  { name = "inserter", rx = 14.5, ry = 1.5, dir = 0, half = nil },
  { name = "transport-belt", rx = 15.5, ry = 0.5, dir = 4, half = nil },
  { name = "transport-belt", rx = 14.5, ry = 0.5, dir = 4, half = nil },
  { name = "inserter", rx = 17.5, ry = 1.5, dir = 0, half = nil },
  { name = "transport-belt", rx = 17.5, ry = 0.5, dir = 4, half = nil },
  { name = "transport-belt", rx = 16.5, ry = 0.5, dir = 4, half = nil },
  { name = "inserter", rx = 18.5, ry = 1.5, dir = 0, half = nil },
  { name = "small-electric-pole", rx = 19.5, ry = 1.5, dir = 0, half = nil },
  { name = "transport-belt", rx = 19.5, ry = 0.5, dir = 4, half = nil },
  { name = "transport-belt", rx = 18.5, ry = 0.5, dir = 4, half = nil },
  { name = "inserter", rx = 21.5, ry = 1.5, dir = 0, half = nil },
  { name = "transport-belt", rx = 21.5, ry = 0.5, dir = 4, half = nil },
  { name = "transport-belt", rx = 20.5, ry = 0.5, dir = 4, half = nil },
  { name = "inserter", rx = 22.5, ry = 1.5, dir = 0, half = nil },
  { name = "transport-belt", rx = 23.5, ry = 0.5, dir = 4, half = nil },
  { name = "transport-belt", rx = 22.5, ry = 0.5, dir = 4, half = nil },
  { name = "inserter", rx = 25.5, ry = 1.5, dir = 0, half = nil },
  { name = "small-electric-pole", rx = 24.5, ry = 1.5, dir = 0, half = nil },
  { name = "transport-belt", rx = 25.5, ry = 0.5, dir = 4, half = nil },
  { name = "transport-belt", rx = 24.5, ry = 0.5, dir = 4, half = nil },
  { name = "inserter", rx = 26.5, ry = 1.5, dir = 0, half = nil },
  { name = "transport-belt", rx = 26.5, ry = 0.5, dir = 4, half = nil },
  { name = "transport-belt", rx = 1.5, ry = 2.5, dir = 0, half = nil },
  { name = "transport-belt", rx = 1.5, ry = 3.5, dir = 0, half = nil },
  { name = "stone-furnace", rx = 5, ry = 3, dir = 0, half = nil },
  { name = "stone-furnace", rx = 7, ry = 3, dir = 0, half = nil },
  { name = "stone-furnace", rx = 9, ry = 3, dir = 0, half = nil },
  { name = "stone-furnace", rx = 11, ry = 3, dir = 0, half = nil },
  { name = "stone-furnace", rx = 13, ry = 3, dir = 0, half = nil },
  { name = "stone-furnace", rx = 15, ry = 3, dir = 0, half = nil },
  { name = "stone-furnace", rx = 17, ry = 3, dir = 0, half = nil },
  { name = "stone-furnace", rx = 19, ry = 3, dir = 0, half = nil },
  { name = "stone-furnace", rx = 21, ry = 3, dir = 0, half = nil },
  { name = "stone-furnace", rx = 23, ry = 3, dir = 0, half = nil },
  { name = "stone-furnace", rx = 25, ry = 3, dir = 0, half = nil },
  { name = "stone-furnace", rx = 27, ry = 3, dir = 0, half = nil },
  { name = "splitter", rx = 0.5, ry = 5, dir = 4, half = nil },
  { name = "transport-belt", rx = 1.5, ry = 5.5, dir = 8, half = nil },
  { name = "transport-belt", rx = 1.5, ry = 4.5, dir = 0, half = nil },
  { name = "splitter", rx = 2.5, ry = 5, dir = 12, half = nil },
  { name = "transport-belt", rx = 3.5, ry = 5.5, dir = 12, half = nil },
  { name = "inserter", rx = 5.5, ry = 4.5, dir = 0, half = nil },
  { name = "transport-belt", rx = 5.5, ry = 5.5, dir = 4, half = nil },
  { name = "inserter", rx = 6.5, ry = 4.5, dir = 0, half = nil },
  { name = "transport-belt", rx = 6.5, ry = 5.5, dir = 4, half = nil },
  { name = "transport-belt", rx = 7.5, ry = 5.5, dir = 4, half = nil },
  { name = "small-electric-pole", rx = 7.5, ry = 4.5, dir = 0, half = nil },
  { name = "inserter", rx = 9.5, ry = 4.5, dir = 0, half = nil },
  { name = "transport-belt", rx = 8.5, ry = 5.5, dir = 4, half = nil },
  { name = "transport-belt", rx = 9.5, ry = 5.5, dir = 4, half = nil },
  { name = "inserter", rx = 10.5, ry = 4.5, dir = 0, half = nil },
  { name = "transport-belt", rx = 10.5, ry = 5.5, dir = 4, half = nil },
  { name = "transport-belt", rx = 11.5, ry = 5.5, dir = 4, half = nil },
  { name = "inserter", rx = 13.5, ry = 4.5, dir = 0, half = nil },
  { name = "transport-belt", rx = 12.5, ry = 5.5, dir = 4, half = nil },
  { name = "transport-belt", rx = 13.5, ry = 5.5, dir = 4, half = nil },
  { name = "small-electric-pole", rx = 12.5, ry = 4.5, dir = 0, half = nil },
  { name = "inserter", rx = 14.5, ry = 4.5, dir = 0, half = nil },
  { name = "transport-belt", rx = 14.5, ry = 5.5, dir = 4, half = nil },
  { name = "transport-belt", rx = 15.5, ry = 5.5, dir = 4, half = nil },
  { name = "inserter", rx = 17.5, ry = 4.5, dir = 0, half = nil },
  { name = "transport-belt", rx = 16.5, ry = 5.5, dir = 4, half = nil },
  { name = "transport-belt", rx = 17.5, ry = 5.5, dir = 4, half = nil },
  { name = "inserter", rx = 18.5, ry = 4.5, dir = 0, half = nil },
  { name = "transport-belt", rx = 18.5, ry = 5.5, dir = 4, half = nil },
  { name = "transport-belt", rx = 19.5, ry = 5.5, dir = 4, half = nil },
  { name = "small-electric-pole", rx = 19.5, ry = 4.5, dir = 0, half = nil },
  { name = "inserter", rx = 21.5, ry = 4.5, dir = 0, half = nil },
  { name = "transport-belt", rx = 20.5, ry = 5.5, dir = 4, half = nil },
  { name = "transport-belt", rx = 21.5, ry = 5.5, dir = 4, half = nil },
  { name = "inserter", rx = 22.5, ry = 4.5, dir = 0, half = nil },
  { name = "transport-belt", rx = 22.5, ry = 5.5, dir = 4, half = nil },
  { name = "transport-belt", rx = 23.5, ry = 5.5, dir = 4, half = nil },
  { name = "inserter", rx = 25.5, ry = 4.5, dir = 0, half = nil },
  { name = "transport-belt", rx = 24.5, ry = 5.5, dir = 4, half = nil },
  { name = "transport-belt", rx = 25.5, ry = 5.5, dir = 4, half = nil },
  { name = "small-electric-pole", rx = 24.5, ry = 4.5, dir = 0, half = nil },
  { name = "inserter", rx = 26.5, ry = 4.5, dir = 0, half = nil },
  { name = "transport-belt", rx = 26.5, ry = 5.5, dir = 4, half = nil },
  { name = "transport-belt", rx = 27.5, ry = 5.5, dir = 4, half = nil },
  { name = "transport-belt", rx = 28.5, ry = 5.5, dir = 4, half = nil },
  { name = "underground-belt", rx = 0.5, ry = 6.5, dir = 4, half = "input" },
  { name = "transport-belt", rx = 1.5, ry = 7.5, dir = 8, half = nil },
  { name = "transport-belt", rx = 1.5, ry = 6.5, dir = 8, half = nil },
  { name = "underground-belt", rx = 2.5, ry = 6.5, dir = 4, half = "output" },
  { name = "transport-belt", rx = 3.5, ry = 6.5, dir = 0, half = nil },
  { name = "inserter", rx = 5.5, ry = 6.5, dir = 8, half = nil },
  { name = "stone-furnace", rx = 5, ry = 8, dir = 0, half = nil },
  { name = "inserter", rx = 6.5, ry = 6.5, dir = 8, half = nil },
  { name = "stone-furnace", rx = 7, ry = 8, dir = 0, half = nil },
  { name = "small-lamp", rx = 7.5, ry = 6.5, dir = 0, half = nil },
  { name = "inserter", rx = 9.5, ry = 6.5, dir = 8, half = nil },
  { name = "stone-furnace", rx = 9, ry = 8, dir = 0, half = nil },
  { name = "inserter", rx = 10.5, ry = 6.5, dir = 8, half = nil },
  { name = "stone-furnace", rx = 11, ry = 8, dir = 0, half = nil },
  { name = "inserter", rx = 13.5, ry = 6.5, dir = 8, half = nil },
  { name = "stone-furnace", rx = 13, ry = 8, dir = 0, half = nil },
  { name = "small-lamp", rx = 12.5, ry = 6.5, dir = 0, half = nil },
  { name = "inserter", rx = 14.5, ry = 6.5, dir = 8, half = nil },
  { name = "stone-furnace", rx = 15, ry = 8, dir = 0, half = nil },
  { name = "inserter", rx = 17.5, ry = 6.5, dir = 8, half = nil },
  { name = "stone-furnace", rx = 17, ry = 8, dir = 0, half = nil },
  { name = "inserter", rx = 18.5, ry = 6.5, dir = 8, half = nil },
  { name = "stone-furnace", rx = 19, ry = 8, dir = 0, half = nil },
  { name = "small-lamp", rx = 19.5, ry = 6.5, dir = 0, half = nil },
  { name = "inserter", rx = 21.5, ry = 6.5, dir = 8, half = nil },
  { name = "stone-furnace", rx = 21, ry = 8, dir = 0, half = nil },
  { name = "inserter", rx = 22.5, ry = 6.5, dir = 8, half = nil },
  { name = "stone-furnace", rx = 23, ry = 8, dir = 0, half = nil },
  { name = "inserter", rx = 25.5, ry = 6.5, dir = 8, half = nil },
  { name = "stone-furnace", rx = 25, ry = 8, dir = 0, half = nil },
  { name = "inserter", rx = 26.5, ry = 6.5, dir = 8, half = nil },
  { name = "stone-furnace", rx = 27, ry = 8, dir = 0, half = nil },
  { name = "transport-belt", rx = 1.5, ry = 9.5, dir = 8, half = nil },
  { name = "transport-belt", rx = 1.5, ry = 8.5, dir = 8, half = nil },
  { name = "small-electric-pole", rx = 2.5, ry = 9.5, dir = 0, half = nil },
  { name = "inserter", rx = 5.5, ry = 9.5, dir = 8, half = nil },
  { name = "inserter", rx = 6.5, ry = 9.5, dir = 8, half = nil },
  { name = "small-electric-pole", rx = 7.5, ry = 9.5, dir = 0, half = nil },
  { name = "inserter", rx = 9.5, ry = 9.5, dir = 8, half = nil },
  { name = "inserter", rx = 10.5, ry = 9.5, dir = 8, half = nil },
  { name = "inserter", rx = 13.5, ry = 9.5, dir = 8, half = nil },
  { name = "small-electric-pole", rx = 12.5, ry = 9.5, dir = 0, half = nil },
  { name = "inserter", rx = 14.5, ry = 9.5, dir = 8, half = nil },
  { name = "inserter", rx = 17.5, ry = 9.5, dir = 8, half = nil },
  { name = "inserter", rx = 18.5, ry = 9.5, dir = 8, half = nil },
  { name = "small-electric-pole", rx = 19.5, ry = 9.5, dir = 0, half = nil },
  { name = "inserter", rx = 21.5, ry = 9.5, dir = 8, half = nil },
  { name = "inserter", rx = 22.5, ry = 9.5, dir = 8, half = nil },
  { name = "inserter", rx = 25.5, ry = 9.5, dir = 8, half = nil },
  { name = "small-electric-pole", rx = 24.5, ry = 9.5, dir = 0, half = nil },
  { name = "inserter", rx = 26.5, ry = 9.5, dir = 8, half = nil },
  { name = "transport-belt", rx = 1.5, ry = 10.5, dir = 4, half = nil },
  { name = "transport-belt", rx = 3.5, ry = 10.5, dir = 4, half = nil },
  { name = "transport-belt", rx = 2.5, ry = 10.5, dir = 4, half = nil },
  { name = "transport-belt", rx = 5.5, ry = 10.5, dir = 4, half = nil },
  { name = "transport-belt", rx = 4.5, ry = 10.5, dir = 4, half = nil },
  { name = "transport-belt", rx = 7.5, ry = 10.5, dir = 4, half = nil },
  { name = "transport-belt", rx = 6.5, ry = 10.5, dir = 4, half = nil },
  { name = "transport-belt", rx = 9.5, ry = 10.5, dir = 4, half = nil },
  { name = "transport-belt", rx = 8.5, ry = 10.5, dir = 4, half = nil },
  { name = "transport-belt", rx = 11.5, ry = 10.5, dir = 4, half = nil },
  { name = "transport-belt", rx = 10.5, ry = 10.5, dir = 4, half = nil },
  { name = "transport-belt", rx = 13.5, ry = 10.5, dir = 4, half = nil },
  { name = "transport-belt", rx = 12.5, ry = 10.5, dir = 4, half = nil },
  { name = "transport-belt", rx = 15.5, ry = 10.5, dir = 4, half = nil },
  { name = "transport-belt", rx = 14.5, ry = 10.5, dir = 4, half = nil },
  { name = "transport-belt", rx = 17.5, ry = 10.5, dir = 4, half = nil },
  { name = "transport-belt", rx = 16.5, ry = 10.5, dir = 4, half = nil },
  { name = "transport-belt", rx = 19.5, ry = 10.5, dir = 4, half = nil },
  { name = "transport-belt", rx = 18.5, ry = 10.5, dir = 4, half = nil },
  { name = "transport-belt", rx = 21.5, ry = 10.5, dir = 4, half = nil },
  { name = "transport-belt", rx = 20.5, ry = 10.5, dir = 4, half = nil },
  { name = "transport-belt", rx = 23.5, ry = 10.5, dir = 4, half = nil },
  { name = "transport-belt", rx = 22.5, ry = 10.5, dir = 4, half = nil },
  { name = "transport-belt", rx = 25.5, ry = 10.5, dir = 4, half = nil },
  { name = "transport-belt", rx = 24.5, ry = 10.5, dir = 4, half = nil },
  { name = "transport-belt", rx = 26.5, ry = 10.5, dir = 4, half = nil },
}
print("EXPECTED_REL entities: " .. tostring(#EXPECTED_REL) .. " (want 179)")

local bots = (type(all_bots) == "table" and #all_bots > 0) and all_bots or { 1 }
print("bots: " .. tostring(#bots))

-- ---------------------------------------------------------------- CHEATS --
print("CHEAT: giving every bot the full material set (87 transport-belt, 48 "
  .. "inserter, 24 stone-furnace, 13 small-electric-pole, 3 small-lamp, 2 "
  .. "splitter, 2 underground-belt) -- gathering is not what this run tests")
for _, b in ipairs(bots) do
  rcon.cheat_item(b, "transport-belt", 87)
  rcon.cheat_item(b, "inserter", 48)
  rcon.cheat_item(b, "stone-furnace", 24)
  rcon.cheat_item(b, "small-electric-pole", 13)
  rcon.cheat_item(b, "small-lamp", 3)
  rcon.cheat_item(b, "splitter", 2)
  rcon.cheat_item(b, "underground-belt", 2)
end

local run_id = record.start({ video = false })
print("recording run " .. run_id)
rcon.sampling_start(run_id)

-- THE THING UNDER TEST: no anchor argument -> Site::Anywhere. Siting picks
-- the anchor; nothing here tells it where to go.
print("planning goal.built(FURNACE_LINE) with NO anchor -- siting chooses...")
local plan_ok, plan = pcall(goal.plan, goal.built(FURNACE_LINE))
if not plan_ok then
  print("PLAN REFUSED: " .. tostring(plan))
  print("This IS a result: siting could not find a clear, ore-legal site for "
    .. "FurnaceLine on this live world. Reported honestly, not worked around.")
  record.milestone_started(1, "FurnaceLine (179 entities) built at a self-chosen site")
  record.milestone_stuck(1, "exhausted", tostring(plan), nil)
  local id = record.finish("incomplete")
  print("RUN FINISHED id=" .. id)
  print("end siting furnace live")
  return
end

print(string.format("PLAN: %d steps, %d bot(s), makespan=%s",
  #plan.steps, #plan.bots, tostring(plan.makespan)))
local placed_plan = plan:count { kind = "place" }
print("planned placements: " .. tostring(placed_plan) .. " (want 179)")

-- Captured into plain tables first: `goal.run` consumes the plan userdata.
-- `s.direction`/`s.underground_half` ARE available on place steps on this
-- branch (unlike Task 4/6's early scripts, written before that field
-- existed) -- see the test `a_place_step_publishes_its_direction_and_
-- underground_half` in crates/scripting_lua/src/globals/goal/plan.rs.
local steps = {}
local per_bot_steps = {}
local per_bot_places = {}
local planned = {} -- place steps only: what THIS plan intends to stand
for i, s in ipairs(plan.steps) do
  local detail
  if s.kind == "walk" then
    detail = string.format("-> (%.2f,%.2f)", s.to.x, s.to.y)
  elseif s.kind == "place" then
    detail = string.format("%s @ (%.2f,%.2f) dir=%s half=%s",
      s.entity, s.pos.x, s.pos.y, tostring(s.direction), tostring(s.underground_half))
    planned[#planned + 1] = {
      entity = s.entity, pos = s.pos, direction = s.direction,
      underground_half = s.underground_half, label = s.label,
    }
    per_bot_places[s.bot] = (per_bot_places[s.bot] or 0) + 1
  else
    detail = s.kind
  end
  steps[i] = { index = i, bot = s.bot, kind = s.kind, id = s.id, detail = detail,
               start = s.start, finish = s.finish }
  per_bot_steps[s.bot] = (per_bot_steps[s.bot] or 0) + 1
end
for _, b in ipairs(bots) do
  print(string.format("bot %s: %s planned step(s), %s placement(s)",
    tostring(b), tostring(per_bot_steps[b] or 0), tostring(per_bot_places[b] or 0)))
end
print("planned placements captured from the plan: " .. tostring(#planned))

-- ------------------------------------- what anchor did siting choose? --
-- Every planned entity's position minus its own blueprint-relative offset
-- should give the SAME anchor -- that agreement is itself evidence siting
-- resolved one consistent site, not something disagreeing internally.
-- EXPECTED_REL is keyed by rounded relative position so this lookup does not
-- depend on plan.steps and EXPECTED_REL sharing an iteration order.
local rel_by_key = {}
for _, e in ipairs(EXPECTED_REL) do
  rel_by_key[string.format("%s|%.1f,%.1f", e.name, e.rx, e.ry)] = e
end

-- Bootstrap the anchor from the bounding boxes (min corner of both sets must
-- correspond to the same physical entity, since the block is a rigid shape).
local rel_min_x, rel_min_y = math.huge, math.huge
for _, e in ipairs(EXPECTED_REL) do
  if e.rx < rel_min_x then rel_min_x = e.rx end
  if e.ry < rel_min_y then rel_min_y = e.ry end
end
local plan_min_x, plan_min_y = math.huge, math.huge
for _, p in ipairs(planned) do
  if p.pos.x < plan_min_x then plan_min_x = p.pos.x end
  if p.pos.y < plan_min_y then plan_min_y = p.pos.y end
end
local anchor = { x = plan_min_x - rel_min_x, y = plan_min_y - rel_min_y }
print(string.format("ANCHOR (recovered from the plan's own bounding box): (%.2f, %.2f)",
  anchor.x, anchor.y))

-- Cross-check: every planned entity, translated back by that one anchor,
-- must name the SAME blueprint entity EXPECTED_REL has at that relative
-- position -- confirming the anchor is consistent across all 179 entities,
-- not just the two used to derive it.
local anchor_agree, anchor_disagree = 0, 0
for _, p in ipairs(planned) do
  local rx, ry = p.pos.x - anchor.x, p.pos.y - anchor.y
  local key = string.format("%s|%.1f,%.1f", p.entity, rx, ry)
  if rel_by_key[key] then
    anchor_agree = anchor_agree + 1
  else
    anchor_disagree = anchor_disagree + 1
    print(string.format("ANCHOR DISAGREEMENT: planned %s @ (%.2f,%.2f) -> relative (%.2f,%.2f) "
      .. "-- no such entity in the blueprint's own geometry at that offset",
      p.entity, p.pos.x, p.pos.y, rx, ry))
  end
end
print(string.format("anchor agreement: %d/%d planned placements consistent with ONE anchor, %d disagree",
  anchor_agree, #planned, anchor_disagree))

-- ------------------------------------ was the footprint actually clear? --
-- Independent of "the plan didn't refuse" -- ask the live surface directly,
-- BEFORE goal.run places anything, whether the chosen footprint holds
-- anything siting should have refused (excluding resources/characters, which
-- `siting_occupant` and `first_obstruction` deliberately do not treat as
-- blocking -- see docs/superpowers/notes/2026-09-06-block-siting.md).
local footprint_center = { x = anchor.x + 14.5, y = anchor.y + 5.5 }
local pre_scan = rcon.find_entities_in_radius(footprint_center, 21)
local IGNORE = {
  ["iron-ore"] = true, ["copper-ore"] = true, ["coal"] = true, ["stone"] = true,
  ["character"] = true,
}
local pre_obstacles = {}
for _, e in ipairs(pre_scan or {}) do
  local lx, ly = e.position.x - anchor.x, e.position.y - anchor.y
  if lx >= -1 and lx <= 30 and ly >= -1 and ly <= 12 and not IGNORE[e.name] then
    pre_obstacles[#pre_obstacles + 1] = e
  end
end
if #pre_obstacles == 0 then
  print("FOOTPRINT CHECK: 0 non-resource, non-character entities found inside the chosen "
    .. "footprint before goal.run -- the site was actually clear, not just unrefused")
else
  print("FOOTPRINT CHECK: " .. tostring(#pre_obstacles) .. " entity(ies) already inside the "
    .. "chosen footprint before goal.run -- see below")
  for _, e in ipairs(pre_obstacles) do
    print(string.format("  pre-existing: %s @ (%.2f,%.2f)", e.name, e.position.x, e.position.y))
  end
end

print("dispatching...")
local tick_before = rcon.game_tick()
local obs = goal.run(plan)
local tick_after = rcon.game_tick()
print(string.format("run returned (tick_before=%s tick_after=%s elapsed_ticks=%s)",
  tostring(tick_before), tostring(tick_after),
  (tick_before and tick_after) and tostring(tick_after - tick_before) or "?"))

local n_actions = record.actions(steps, obs.actions)
local n_walks = record.walks(obs.walks)
local n_teleports = record.teleports()
local n_refusals = record.refusals()
local n_enclosures = record.enclosures()
print(string.format("recorded: %d action events, %d walk events, %d teleports, %d refusals, %d enclosures",
  n_actions, n_walks, n_teleports, n_refusals, n_enclosures))
if n_enclosures > 0 then print("WALLED IN: see record.enclosures() above") end

record.milestone_started(1, string.format(
  "FurnaceLine (179 entities) built at self-sited anchor (%.1f,%.1f)", anchor.x, anchor.y))
if obs.done and (obs.failed or 0) == 0 and (obs.lost or 0) == 0 then
  record.milestone_satisfied(1, 1, "plan_empty")
else
  record.milestone_stuck(1, obs.done and "stuck" or "exhausted", obs.first_error, nil)
end

print(string.format("done=%s success=%s failed=%s lost=%s pending=%s running=%s",
  tostring(obs.done), tostring(obs.success), tostring(obs.failed),
  tostring(obs.lost), tostring(obs.pending), tostring(obs.running)))
print("first_error: " .. tostring(obs.first_error))
if (obs.failed or 0) > 0 or (obs.lost or 0) > 0 then
  local fs = obs:failures()
  for i = 1, #fs do
    print(string.format("failure[%d] id=%s status=%s attempts=%s error=%s",
      i, tostring(fs[i].id), tostring(fs[i].status), tostring(fs[i].attempts), tostring(fs[i].error)))
  end
end

-- ------------------------------------------------ read the world back --
-- The deliverable: for every one of the (up to) 179 planned entities, is
-- something standing at the expected tile, with the expected NAME, facing
-- the expected DIRECTION, and (for the two underground belts) the expected
-- HALF -- read from the live surface via rcon.find_entities_in_radius, not
-- from the record the executor kept of its own beliefs.
print("---- reading the world back: name + position + direction + half, per entity ----")
local NAMES = { "transport-belt", "inserter", "stone-furnace", "small-electric-pole",
  "small-lamp", "splitter", "underground-belt" }
local actual_by_key = {}
local total_found = 0
for _, name in ipairs(NAMES) do
  local found = rcon.find_entities_in_radius(footprint_center, 21, name)
  print(name .. ": " .. tostring(#found) .. " found near the chosen anchor")
  for _, e in ipairs(found or {}) do
    total_found = total_found + 1
    local key = string.format("%.1f,%.1f", e.position.x, e.position.y)
    actual_by_key[key] = { name = e.name, position = e.position, direction = e.direction,
      underground_half = e.underground_half }
  end
end
print("total entities found near the chosen anchor: " .. tostring(total_found) .. " (want 179)")

local matched, wrong_dir, wrong_half, missing = 0, 0, 0, 0
for _, exp in ipairs(planned) do
  local key = string.format("%.1f,%.1f", exp.pos.x, exp.pos.y)
  local act = actual_by_key[key]
  if act == nil then
    missing = missing + 1
    print(string.format("MISSING: %s expected @ (%s) dir=%s -- nothing found there",
      exp.entity, key, tostring(exp.direction)))
  elseif act.name ~= exp.entity then
    missing = missing + 1
    print(string.format("WRONG ENTITY: expected %s @ (%s), found %s", exp.entity, key, act.name))
  elseif tostring(act.direction) ~= tostring(exp.direction) then
    wrong_dir = wrong_dir + 1
    print(string.format("WRONG DIRECTION: %s @ (%s) expected dir=%s, game reports dir=%s",
      exp.entity, key, tostring(exp.direction), tostring(act.direction)))
  elseif exp.underground_half ~= nil and tostring(act.underground_half) ~= tostring(exp.underground_half) then
    wrong_half = wrong_half + 1
    print(string.format("WRONG HALF: %s @ (%s) expected half=%s, game reports half=%s",
      exp.entity, key, tostring(exp.underground_half), tostring(act.underground_half)))
  else
    matched = matched + 1
  end
end
print(string.format(
  "VERDICT: %d/%d planned placements matched name+position+direction(+half), %d wrong direction, "
    .. "%d wrong half, %d missing/wrong entity",
  matched, #planned, wrong_dir, wrong_half, missing))

print("---- underground pair, read off the surface ----")
local underground_actual = {}
for key, a in pairs(actual_by_key) do
  if a.name == "underground-belt" then
    underground_actual[#underground_actual + 1] = a
  end
end
table.sort(underground_actual, function(a, b) return a.position.x < b.position.x end)
print("underground-belt entities found: " .. tostring(#underground_actual) .. " (want 2)")
for _, u in ipairs(underground_actual) do
  print(string.format("  underground-belt @ (%.2f,%.2f) dir=%s underground_half=%s",
    u.position.x, u.position.y, tostring(u.direction), tostring(u.underground_half)))
end

print("")
print("REMINDER: this run proves STANDING, not MOVING. FurnaceLine carries 13 "
  .. "small-electric-pole and NO GENERATOR. The 87 belts, 48 inserters, 2 "
  .. "splitters and 2 underground belts checked above are proven only to "
  .. "stand at the right tile facing the right way -- nothing here shows "
  .. "any of them move a single item, and nothing here could.")

local id = record.finish(obs.done and "done" or "incomplete")
print("RUN FINISHED id=" .. id)
print("end siting furnace live")

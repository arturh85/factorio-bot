-- Task 6: build FurnaceLine (rcontest.lua) live -- 179 entities: 87
-- transport-belt, 48 inserter, 24 stone-furnace, 13 small-electric-pole, 3
-- small-lamp, 2 splitter, 2 underground-belt. Fifty times MinerLine/Task4.
--
-- CHEATED, DISCLOSED: every bot is given the full material set (87
-- transport-belt, 48 inserter, 24 stone-furnace, 13 small-electric-pole, 3
-- small-lamp, 2 splitter, 2 underground-belt) via rcon.cheat_item before
-- planning -- gathering is out of scope, same as Task 4/5. No entity is
-- ever cheated in: every placement below is a real goal.run dispatch.
--
-- SITING: this blueprint's decoded footprint is 29x11 tiles (x in
-- [0.5,28.5], y in [0.5,10.5]), computed offline from the same base64/zlib
-- blob rcontest.lua embeds. The anchor below was picked after scanning a
-- circle covering that rectangle for entities (trees/rocks) -- water/cliffs
-- are NOT visible to rcon.find_entities_in_radius (established in Task 4's
-- note) so are not, and cannot be, checked this way. Any tree/rock found is
-- REMOVED BY SCRIPT before planning, disclosed here: this isolates "does a
-- 179-entity block build and smelt" from "can we site one automatically",
-- per the brief. If goal.plan still refuses (e.g. water under the
-- footprint), that is reported as the same siting finding as MinerLine, not
-- worked around further.
--
-- SMELT TEST: FurnaceLine has no output chest -- Counter of its 179
-- entities is {belt:87, inserter:48, furnace:24, pole:13, lamp:3,
-- splitter:2, underground-belt:2}, no container. Ore/fuel supply via belts
-- is explicitly out of this whole sub-project's scope (spec's own
-- self-review). So ore and coal are inserted DIRECTLY into each furnace's
-- source/fuel inventory via rcon.insert_to_inventory -- bypassing the belt
-- supply network on purpose, disclosed -- which tests ONE part of what is in
-- scope: do the 24 built furnaces actually smelt.
--
-- IT CANNOT TEST THE INSERTERS, and an earlier version of this comment
-- claimed it did. Two reasons, both fatal:
--   1. Plates are counted by summing each furnace's OWN output_inventory. If
--      the output inserters were removing them that number would FLATTEN --
--      a rising curve is evidence that nothing is emptying the furnaces.
--   2. FurnaceLine has 13 small-electric-pole and NO GENERATOR, and this
--      script cheats in no power source. Inserters are electric. Power
--      coverage is not power capacity: an unpowered inserter does not run
--      slowly, it does not run at all.
-- So the 87 belts, 48 inserters, 2 splitters and 2 underground belts are
-- checked here only for STANDING correctly (position + direction + half, read
-- back off the surface below). Nothing in this script shows them moving an
-- item, and nothing in it could.
print("start furnace run")

local FURNACE_LINE = "0eNqdm81vo0gQxf+ViLOd0NUf0D7ubTXSXuYwh9VoRJweL7sYEODsRlH+98WxZmKNaXiPk5UPfn5Vpqq7X+HX5LE6hbYr6yHZvSZPod93ZTuUTZ3sks/NqduHu7+Goe13Dw/fi/3QdGXz/t/9/b45PjyX4d+H7aemOv1h/37+cvg9DYeXfz59+5Jskr4u2u3QbA9d+XRm/5fsxG+Sl2Sn1NsmKR778bIhbM//15b1IdkN3SlsknLf1H2y+/M16ctDXVTna4eXNoyCyiEcR3JdHM8/DV1R923TDdvHUA3JyCzrpzC+zYhfvLgfmjpsv5+6utiHq2sFuHbftG3otm1VDNeXauDSsmvqmwvN29dNEuqhHMpwifz9h5dv9en4GLoxoFjMm6Rt+vLycb1nWN3b9xSn93bkP5Vd2F/++h7XL1ihsePr2wRIwyDN6DMwVhis/fg06j50w/i7G6CdD9jByiyjLIOxhsHmQMBuPmD/UTjHoqq2oRrfsCv327apwi0tm6epFA40YwJVeKE4iitABv1CzHiReEobXiU5xUXKRKULQeOFoqjWpfBS+akRAyPFovRC2GS5KJnnCV4viuqwQqwsVI8VpGKUWQgbLxlF9VnBa0ZRnVagolnojkIUDdUehSgaqj8KVDT5Qths0Sw0XE0UDdVxNVE0VMvVSNHI0l4MLxqhWq4mtmNUy9VI0chCi9R40QjVIjVeNEK1SI0UjSxsRDVZNLLQcg1eNEK1XIMXjVAt10BFs7C9NUTRUJ3RGPqMJRGJlibpCMlFTsG3h5cLZhKSoZBsBpKjED8D8SjkfDyIUWwKU/QMRcGUmdxagSkzybUapsxk1xqUInPZtTBlLrvwjStz2YXvXJnL7tWt21blMNl2fiyDiCnh6fK2vzYgM+VUpDTXRDwPBUQs0xG7KZ7QrtFNxJNcTfg7sVgNbfBYZDlwlrBiYtocbXJg2jLalMG4+TrvKBa+J7yYCCNLac8ECjVTtMeDcYXxYmJBa94ywdQZ3uTBwJbxYmJhO97iwNRlvCmDgfOV7lEsA57xYiKQPOUtEyjaXPEmDwYWxouJha15ywRTZ3iTBwNbxouJhe14iwNTl/GmDAbOV7pHsQx4xouJQHzKWyZQtF7xJg8GFsaLiYWtecsEU2d4kwcDW8aLiYXteIsDU5fxpgwGzle6R7EMeMaLMbHhYMqbMVC4KiXKJuPI+GFGco78UUyn+il0h64ZXyPsH03E3aA3P59HqNvT+WGJiXfivaoMOYKqlLeuHAZ2RG5kOTfNaYgmJ6NPqy52f+fEARVMhOcsvXxS2NVYfvl0iglTirMJI8Lklz5VFcc2fnKMJV5p4ugIxmc4BzMSHzVrB5U50haNSMuYMxgoLSe91og0D90VShZuC0mZ8xEWorBG8HSIQp1hQGmadJcj0gxzzgClWdKyjkhz2I3hl26MjDkDgCHmpJ8eCdEz+3RMmk5Jk35a2tV4HNhLg9KEdP4j0jSzPwWlGXKcEJHG75E8ps/R4BwDZ+Th4Ur2JI/ZFoGxe2JDgyGvBuSUmx2L2ihiTwJKpNxikKmZbQDINCt9zmguLbOQgyIdswKDzIxZOkHmWv8rmkzKAMNE2pRZtUAmtdyATFnpi8SSaamFBxTJH9cV+OSvpc+6KNnRj/Sj5Iye2KLknH4qHyV7euIKkompvyPJ/IgTJQs9lEXJmh9QomjDD1VRtOXniyh6xUwURWf8OA9F5/wIEkV7fhoHoolnDRRZi8TjBoosxkz4ASCKXvGAMope8Ygyirb86AlFO35chqJXTI5QdM5Pu1C05wdAU+ivm8u3L3dXX3LdJFUxosbf6fu734q+3N99Po7o8xdQN8lz6PrLxfm4Szc+c5lKnXVvb/8DoD9BzA=="

-- Decoded offline: x in [0.5,28.5], y in [0.5,10.5] (29x11 footprint).
local ANCHOR = { x = 60, y = -100 } -- rescanned clean (0 obstacles) after (40,40) refused a furnace tile

-- All 179 entities, decoded OFFLINE from the same base64/zlib blob above
-- (blueprint version 281474976710656 = 1.0.0.0, so raw pre-2.0 directions
-- are doubled onto the 16-point scale: dir = (raw * 2) % 16 -- matching
-- import_stack's own migration). Written because `plan.steps[i].direction`
-- did not exist when this run was made: `ActionKind::Place` in
-- crates/scripting_lua/src/globals/goal/plan.rs set only `kind`, `entity`,
-- `pos`, so `s.direction` was always Lua `nil` and a verdict built on it
-- would have reported "wrong direction" for every entity regardless of what
-- the game did. **That gap is closed** -- a place step now publishes
-- `direction` and `underground_half` -- so a future script should read them
-- from the plan rather than re-deriving the migration by hand as this table
-- does. It is kept as-is because it is the ground truth THIS run's numbers
-- were measured against, and rewriting it would invalidate them.
local EXPECTED = {
  { name = "transport-belt", x = 1.5, y = 0.5, dir = 4, half = nil },
  { name = "transport-belt", x = 1.5, y = 1.5, dir = 0, half = nil },
  { name = "transport-belt", x = 3.5, y = 0.5, dir = 4, half = nil },
  { name = "transport-belt", x = 2.5, y = 0.5, dir = 4, half = nil },
  { name = "inserter", x = 5.5, y = 1.5, dir = 0, half = nil },
  { name = "transport-belt", x = 5.5, y = 0.5, dir = 4, half = nil },
  { name = "transport-belt", x = 4.5, y = 0.5, dir = 4, half = nil },
  { name = "inserter", x = 6.5, y = 1.5, dir = 0, half = nil },
  { name = "small-electric-pole", x = 7.5, y = 1.5, dir = 0, half = nil },
  { name = "transport-belt", x = 7.5, y = 0.5, dir = 4, half = nil },
  { name = "transport-belt", x = 6.5, y = 0.5, dir = 4, half = nil },
  { name = "inserter", x = 9.5, y = 1.5, dir = 0, half = nil },
  { name = "transport-belt", x = 9.5, y = 0.5, dir = 4, half = nil },
  { name = "transport-belt", x = 8.5, y = 0.5, dir = 4, half = nil },
  { name = "inserter", x = 10.5, y = 1.5, dir = 0, half = nil },
  { name = "transport-belt", x = 11.5, y = 0.5, dir = 4, half = nil },
  { name = "transport-belt", x = 10.5, y = 0.5, dir = 4, half = nil },
  { name = "inserter", x = 13.5, y = 1.5, dir = 0, half = nil },
  { name = "small-electric-pole", x = 12.5, y = 1.5, dir = 0, half = nil },
  { name = "transport-belt", x = 13.5, y = 0.5, dir = 4, half = nil },
  { name = "transport-belt", x = 12.5, y = 0.5, dir = 4, half = nil },
  { name = "inserter", x = 14.5, y = 1.5, dir = 0, half = nil },
  { name = "transport-belt", x = 15.5, y = 0.5, dir = 4, half = nil },
  { name = "transport-belt", x = 14.5, y = 0.5, dir = 4, half = nil },
  { name = "inserter", x = 17.5, y = 1.5, dir = 0, half = nil },
  { name = "transport-belt", x = 17.5, y = 0.5, dir = 4, half = nil },
  { name = "transport-belt", x = 16.5, y = 0.5, dir = 4, half = nil },
  { name = "inserter", x = 18.5, y = 1.5, dir = 0, half = nil },
  { name = "small-electric-pole", x = 19.5, y = 1.5, dir = 0, half = nil },
  { name = "transport-belt", x = 19.5, y = 0.5, dir = 4, half = nil },
  { name = "transport-belt", x = 18.5, y = 0.5, dir = 4, half = nil },
  { name = "inserter", x = 21.5, y = 1.5, dir = 0, half = nil },
  { name = "transport-belt", x = 21.5, y = 0.5, dir = 4, half = nil },
  { name = "transport-belt", x = 20.5, y = 0.5, dir = 4, half = nil },
  { name = "inserter", x = 22.5, y = 1.5, dir = 0, half = nil },
  { name = "transport-belt", x = 23.5, y = 0.5, dir = 4, half = nil },
  { name = "transport-belt", x = 22.5, y = 0.5, dir = 4, half = nil },
  { name = "inserter", x = 25.5, y = 1.5, dir = 0, half = nil },
  { name = "small-electric-pole", x = 24.5, y = 1.5, dir = 0, half = nil },
  { name = "transport-belt", x = 25.5, y = 0.5, dir = 4, half = nil },
  { name = "transport-belt", x = 24.5, y = 0.5, dir = 4, half = nil },
  { name = "inserter", x = 26.5, y = 1.5, dir = 0, half = nil },
  { name = "transport-belt", x = 26.5, y = 0.5, dir = 4, half = nil },
  { name = "transport-belt", x = 1.5, y = 2.5, dir = 0, half = nil },
  { name = "transport-belt", x = 1.5, y = 3.5, dir = 0, half = nil },
  { name = "stone-furnace", x = 5, y = 3, dir = 0, half = nil },
  { name = "stone-furnace", x = 7, y = 3, dir = 0, half = nil },
  { name = "stone-furnace", x = 9, y = 3, dir = 0, half = nil },
  { name = "stone-furnace", x = 11, y = 3, dir = 0, half = nil },
  { name = "stone-furnace", x = 13, y = 3, dir = 0, half = nil },
  { name = "stone-furnace", x = 15, y = 3, dir = 0, half = nil },
  { name = "stone-furnace", x = 17, y = 3, dir = 0, half = nil },
  { name = "stone-furnace", x = 19, y = 3, dir = 0, half = nil },
  { name = "stone-furnace", x = 21, y = 3, dir = 0, half = nil },
  { name = "stone-furnace", x = 23, y = 3, dir = 0, half = nil },
  { name = "stone-furnace", x = 25, y = 3, dir = 0, half = nil },
  { name = "stone-furnace", x = 27, y = 3, dir = 0, half = nil },
  { name = "splitter", x = 0.5, y = 5, dir = 4, half = nil },
  { name = "transport-belt", x = 1.5, y = 5.5, dir = 8, half = nil },
  { name = "transport-belt", x = 1.5, y = 4.5, dir = 0, half = nil },
  { name = "splitter", x = 2.5, y = 5, dir = 12, half = nil },
  { name = "transport-belt", x = 3.5, y = 5.5, dir = 12, half = nil },
  { name = "inserter", x = 5.5, y = 4.5, dir = 0, half = nil },
  { name = "transport-belt", x = 5.5, y = 5.5, dir = 4, half = nil },
  { name = "inserter", x = 6.5, y = 4.5, dir = 0, half = nil },
  { name = "transport-belt", x = 6.5, y = 5.5, dir = 4, half = nil },
  { name = "transport-belt", x = 7.5, y = 5.5, dir = 4, half = nil },
  { name = "small-electric-pole", x = 7.5, y = 4.5, dir = 0, half = nil },
  { name = "inserter", x = 9.5, y = 4.5, dir = 0, half = nil },
  { name = "transport-belt", x = 8.5, y = 5.5, dir = 4, half = nil },
  { name = "transport-belt", x = 9.5, y = 5.5, dir = 4, half = nil },
  { name = "inserter", x = 10.5, y = 4.5, dir = 0, half = nil },
  { name = "transport-belt", x = 10.5, y = 5.5, dir = 4, half = nil },
  { name = "transport-belt", x = 11.5, y = 5.5, dir = 4, half = nil },
  { name = "inserter", x = 13.5, y = 4.5, dir = 0, half = nil },
  { name = "transport-belt", x = 12.5, y = 5.5, dir = 4, half = nil },
  { name = "transport-belt", x = 13.5, y = 5.5, dir = 4, half = nil },
  { name = "small-electric-pole", x = 12.5, y = 4.5, dir = 0, half = nil },
  { name = "inserter", x = 14.5, y = 4.5, dir = 0, half = nil },
  { name = "transport-belt", x = 14.5, y = 5.5, dir = 4, half = nil },
  { name = "transport-belt", x = 15.5, y = 5.5, dir = 4, half = nil },
  { name = "inserter", x = 17.5, y = 4.5, dir = 0, half = nil },
  { name = "transport-belt", x = 16.5, y = 5.5, dir = 4, half = nil },
  { name = "transport-belt", x = 17.5, y = 5.5, dir = 4, half = nil },
  { name = "inserter", x = 18.5, y = 4.5, dir = 0, half = nil },
  { name = "transport-belt", x = 18.5, y = 5.5, dir = 4, half = nil },
  { name = "transport-belt", x = 19.5, y = 5.5, dir = 4, half = nil },
  { name = "small-electric-pole", x = 19.5, y = 4.5, dir = 0, half = nil },
  { name = "inserter", x = 21.5, y = 4.5, dir = 0, half = nil },
  { name = "transport-belt", x = 20.5, y = 5.5, dir = 4, half = nil },
  { name = "transport-belt", x = 21.5, y = 5.5, dir = 4, half = nil },
  { name = "inserter", x = 22.5, y = 4.5, dir = 0, half = nil },
  { name = "transport-belt", x = 22.5, y = 5.5, dir = 4, half = nil },
  { name = "transport-belt", x = 23.5, y = 5.5, dir = 4, half = nil },
  { name = "inserter", x = 25.5, y = 4.5, dir = 0, half = nil },
  { name = "transport-belt", x = 24.5, y = 5.5, dir = 4, half = nil },
  { name = "transport-belt", x = 25.5, y = 5.5, dir = 4, half = nil },
  { name = "small-electric-pole", x = 24.5, y = 4.5, dir = 0, half = nil },
  { name = "inserter", x = 26.5, y = 4.5, dir = 0, half = nil },
  { name = "transport-belt", x = 26.5, y = 5.5, dir = 4, half = nil },
  { name = "transport-belt", x = 27.5, y = 5.5, dir = 4, half = nil },
  { name = "transport-belt", x = 28.5, y = 5.5, dir = 4, half = nil },
  { name = "underground-belt", x = 0.5, y = 6.5, dir = 4, half = "input" },
  { name = "transport-belt", x = 1.5, y = 7.5, dir = 8, half = nil },
  { name = "transport-belt", x = 1.5, y = 6.5, dir = 8, half = nil },
  { name = "underground-belt", x = 2.5, y = 6.5, dir = 4, half = "output" },
  { name = "transport-belt", x = 3.5, y = 6.5, dir = 0, half = nil },
  { name = "inserter", x = 5.5, y = 6.5, dir = 8, half = nil },
  { name = "stone-furnace", x = 5, y = 8, dir = 0, half = nil },
  { name = "inserter", x = 6.5, y = 6.5, dir = 8, half = nil },
  { name = "stone-furnace", x = 7, y = 8, dir = 0, half = nil },
  { name = "small-lamp", x = 7.5, y = 6.5, dir = 0, half = nil },
  { name = "inserter", x = 9.5, y = 6.5, dir = 8, half = nil },
  { name = "stone-furnace", x = 9, y = 8, dir = 0, half = nil },
  { name = "inserter", x = 10.5, y = 6.5, dir = 8, half = nil },
  { name = "stone-furnace", x = 11, y = 8, dir = 0, half = nil },
  { name = "inserter", x = 13.5, y = 6.5, dir = 8, half = nil },
  { name = "stone-furnace", x = 13, y = 8, dir = 0, half = nil },
  { name = "small-lamp", x = 12.5, y = 6.5, dir = 0, half = nil },
  { name = "inserter", x = 14.5, y = 6.5, dir = 8, half = nil },
  { name = "stone-furnace", x = 15, y = 8, dir = 0, half = nil },
  { name = "inserter", x = 17.5, y = 6.5, dir = 8, half = nil },
  { name = "stone-furnace", x = 17, y = 8, dir = 0, half = nil },
  { name = "inserter", x = 18.5, y = 6.5, dir = 8, half = nil },
  { name = "stone-furnace", x = 19, y = 8, dir = 0, half = nil },
  { name = "small-lamp", x = 19.5, y = 6.5, dir = 0, half = nil },
  { name = "inserter", x = 21.5, y = 6.5, dir = 8, half = nil },
  { name = "stone-furnace", x = 21, y = 8, dir = 0, half = nil },
  { name = "inserter", x = 22.5, y = 6.5, dir = 8, half = nil },
  { name = "stone-furnace", x = 23, y = 8, dir = 0, half = nil },
  { name = "inserter", x = 25.5, y = 6.5, dir = 8, half = nil },
  { name = "stone-furnace", x = 25, y = 8, dir = 0, half = nil },
  { name = "inserter", x = 26.5, y = 6.5, dir = 8, half = nil },
  { name = "stone-furnace", x = 27, y = 8, dir = 0, half = nil },
  { name = "transport-belt", x = 1.5, y = 9.5, dir = 8, half = nil },
  { name = "transport-belt", x = 1.5, y = 8.5, dir = 8, half = nil },
  { name = "small-electric-pole", x = 2.5, y = 9.5, dir = 0, half = nil },
  { name = "inserter", x = 5.5, y = 9.5, dir = 8, half = nil },
  { name = "inserter", x = 6.5, y = 9.5, dir = 8, half = nil },
  { name = "small-electric-pole", x = 7.5, y = 9.5, dir = 0, half = nil },
  { name = "inserter", x = 9.5, y = 9.5, dir = 8, half = nil },
  { name = "inserter", x = 10.5, y = 9.5, dir = 8, half = nil },
  { name = "inserter", x = 13.5, y = 9.5, dir = 8, half = nil },
  { name = "small-electric-pole", x = 12.5, y = 9.5, dir = 0, half = nil },
  { name = "inserter", x = 14.5, y = 9.5, dir = 8, half = nil },
  { name = "inserter", x = 17.5, y = 9.5, dir = 8, half = nil },
  { name = "inserter", x = 18.5, y = 9.5, dir = 8, half = nil },
  { name = "small-electric-pole", x = 19.5, y = 9.5, dir = 0, half = nil },
  { name = "inserter", x = 21.5, y = 9.5, dir = 8, half = nil },
  { name = "inserter", x = 22.5, y = 9.5, dir = 8, half = nil },
  { name = "inserter", x = 25.5, y = 9.5, dir = 8, half = nil },
  { name = "small-electric-pole", x = 24.5, y = 9.5, dir = 0, half = nil },
  { name = "inserter", x = 26.5, y = 9.5, dir = 8, half = nil },
  { name = "transport-belt", x = 1.5, y = 10.5, dir = 4, half = nil },
  { name = "transport-belt", x = 3.5, y = 10.5, dir = 4, half = nil },
  { name = "transport-belt", x = 2.5, y = 10.5, dir = 4, half = nil },
  { name = "transport-belt", x = 5.5, y = 10.5, dir = 4, half = nil },
  { name = "transport-belt", x = 4.5, y = 10.5, dir = 4, half = nil },
  { name = "transport-belt", x = 7.5, y = 10.5, dir = 4, half = nil },
  { name = "transport-belt", x = 6.5, y = 10.5, dir = 4, half = nil },
  { name = "transport-belt", x = 9.5, y = 10.5, dir = 4, half = nil },
  { name = "transport-belt", x = 8.5, y = 10.5, dir = 4, half = nil },
  { name = "transport-belt", x = 11.5, y = 10.5, dir = 4, half = nil },
  { name = "transport-belt", x = 10.5, y = 10.5, dir = 4, half = nil },
  { name = "transport-belt", x = 13.5, y = 10.5, dir = 4, half = nil },
  { name = "transport-belt", x = 12.5, y = 10.5, dir = 4, half = nil },
  { name = "transport-belt", x = 15.5, y = 10.5, dir = 4, half = nil },
  { name = "transport-belt", x = 14.5, y = 10.5, dir = 4, half = nil },
  { name = "transport-belt", x = 17.5, y = 10.5, dir = 4, half = nil },
  { name = "transport-belt", x = 16.5, y = 10.5, dir = 4, half = nil },
  { name = "transport-belt", x = 19.5, y = 10.5, dir = 4, half = nil },
  { name = "transport-belt", x = 18.5, y = 10.5, dir = 4, half = nil },
  { name = "transport-belt", x = 21.5, y = 10.5, dir = 4, half = nil },
  { name = "transport-belt", x = 20.5, y = 10.5, dir = 4, half = nil },
  { name = "transport-belt", x = 23.5, y = 10.5, dir = 4, half = nil },
  { name = "transport-belt", x = 22.5, y = 10.5, dir = 4, half = nil },
  { name = "transport-belt", x = 25.5, y = 10.5, dir = 4, half = nil },
  { name = "transport-belt", x = 24.5, y = 10.5, dir = 4, half = nil },
  { name = "transport-belt", x = 26.5, y = 10.5, dir = 4, half = nil },
}

local bots = (type(all_bots) == "table" and #all_bots > 0) and all_bots or { 1 }
print("bots: " .. tostring(#bots))

-- ------------------------------------------------ SITE SCAN + CLEARING --
print("---- scanning footprint for obstacles (trees/rocks) ----")
local center = { x = ANCHOR.x + 14.5, y = ANCHOR.y + 5.5 }
local scan = rcon.find_entities_in_radius(center, 17)
print("entities found in scan circle: " .. tostring(#scan))
local IGNORE = {
  ["iron-ore"] = true, ["copper-ore"] = true, ["coal"] = true, ["stone"] = true,
  ["character"] = true,
}
local obstacles = {}
for _, e in ipairs(scan or {}) do
  -- Only entities whose position actually falls inside the 29x11 rectangle
  -- (the scan circle is bigger than the footprint on purpose, to be sure
  -- nothing near the edge is missed).
  local lx, ly = e.position.x - ANCHOR.x, e.position.y - ANCHOR.y
  if lx >= -1 and lx <= 30 and ly >= -1 and ly <= 12 and not IGNORE[e.name] then
    obstacles[#obstacles + 1] = e
    print(string.format("  obstacle: %s @ (%.2f,%.2f)", e.name, e.position.x, e.position.y))
  end
end
print("obstacles in footprint: " .. tostring(#obstacles))
if #obstacles > 0 then
  print("CLEARING: mining " .. tostring(#obstacles) .. " obstacle(s) with bot " .. tostring(bots[1])
    .. " before planning -- disclosed, per the brief (\"you may remove trees and rocks by script\")")
  for _, e in ipairs(obstacles) do
    local ok, err = pcall(function() rcon.mine(bots[1], e.name, e.position, 1) end)
    if not ok then print("  mine failed for " .. e.name .. ": " .. tostring(err)) end
  end
end

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

local plan = goal.plan(goal.built(FURNACE_LINE, ANCHOR))
print(string.format("PLAN: %d steps, %d bot(s), makespan=%s",
  #plan.steps, #plan.bots, tostring(plan.makespan)))

local placed_plan = plan:count { kind = "place" }
print("planned placements: " .. tostring(placed_plan) .. " (want 179)")

local steps = {}
local per_bot_steps = {}
local per_bot_places = {}
local expected = {}
for i, s in ipairs(plan.steps) do
  local detail
  if s.kind == "walk" then
    detail = string.format("-> (%.2f,%.2f)", s.to.x, s.to.y)
  elseif s.kind == "place" then
    detail = string.format("%s @ (%.2f,%.2f) dir=%s", s.entity, s.pos.x, s.pos.y, tostring(s.direction))
    expected[#expected + 1] = { entity = s.entity, pos = s.pos, direction = s.direction }
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
print("expected placements captured from the plan: " .. tostring(#expected))

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

record.milestone_started(1, "FurnaceLine (179 entities) built at " .. ANCHOR.x .. "," .. ANCHOR.y)
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
print("---- reading the world back: position + direction, per entity ----")
print("(compared against EXPECTED, decoded offline -- NOT plan.steps, which carries no direction field)")
local NAMES = { "transport-belt", "inserter", "stone-furnace", "small-electric-pole",
  "small-lamp", "splitter", "underground-belt" }
local actual_by_key = {}
local total_found = 0
local underground_actual = {}
for _, name in ipairs(NAMES) do
  local found = rcon.find_entities_in_radius(center, 20, name)
  print(name .. ": " .. tostring(#found) .. " found near anchor")
  for _, e in ipairs(found or {}) do
    total_found = total_found + 1
    local key = string.format("%.1f,%.1f", e.position.x, e.position.y)
    actual_by_key[key] = { name = e.name, position = e.position, direction = e.direction,
      underground_half = e.underground_half }
    if name == "underground-belt" then
      underground_actual[#underground_actual + 1] = {
        position = e.position, direction = e.direction, underground_half = e.underground_half,
      }
    end
  end
end
print("total entities found near anchor: " .. tostring(total_found) .. " (want 179)")

local matched, wrong_dir, missing = 0, 0, 0
for _, exp in ipairs(EXPECTED) do
  local ax, ay = ANCHOR.x + exp.x, ANCHOR.y + exp.y
  local key = string.format("%.1f,%.1f", ax, ay)
  local act = actual_by_key[key]
  if act == nil then
    missing = missing + 1
    print(string.format("MISSING: %s expected @ (%s) dir=%s -- nothing found there",
      exp.name, key, tostring(exp.dir)))
  elseif act.name ~= exp.name then
    missing = missing + 1
    print(string.format("WRONG ENTITY: expected %s @ (%s), found %s", exp.name, key, act.name))
  elseif tostring(act.direction) ~= tostring(exp.dir) then
    wrong_dir = wrong_dir + 1
    print(string.format("WRONG DIRECTION: %s @ (%s) expected dir=%s, game reports dir=%s",
      exp.name, key, tostring(exp.dir), tostring(act.direction)))
  else
    matched = matched + 1
  end
end
print(string.format("VERDICT: %d/%d matched position+direction, %d wrong direction, %d missing/wrong entity",
  matched, #EXPECTED, wrong_dir, missing))

print("---- underground pair, read off the surface (underground_half) ----")
print("underground-belt entities found: " .. tostring(#underground_actual) .. " (want 2)")
table.sort(underground_actual, function(a, b) return a.position.x < b.position.x end)
for _, u in ipairs(underground_actual) do
  print(string.format("  underground-belt @ (%.2f,%.2f) dir=%s underground_half=%s",
    u.position.x, u.position.y, tostring(u.direction), tostring(u.underground_half)))
end
for _, exp in ipairs(EXPECTED) do
  if exp.half ~= nil then
    print(string.format("  EXPECTED underground-belt @ (%.2f,%.2f) dir=%s half=%s",
      ANCHOR.x + exp.x, ANCHOR.y + exp.y, tostring(exp.dir), tostring(exp.half)))
  end
end

-- --------------------------------------------- smelt test: feed furnaces --
-- No output chest exists in this blueprint; belt/ore supply is out of this
-- sub-project's scope (spec's own self-review names "material supply, belts
-- and joins" as out of scope for the whole sub-project). So ore+fuel are
-- inserted DIRECTLY into each built furnace's inventories via
-- rcon.insert_to_inventory, bypassing the belt network -- disclosed above
-- and here again at the point it happens. `insert_to_inventory` moves items
-- OUT OF THE CALLING PLAYER'S OWN INVENTORY (mods/BotBridge/control.lua's
-- `rcon_insert_to_inventory` reads `player.get_item_count` and then
-- `player.remove_item`), so bot 1 is cheated iron-ore/coal first -- the same
-- "cheat materials, never entities" pattern as the placement cheats above.
--
-- Inventory indices: `runtime-api.json`'s `inventory` enum lists entries
-- ALPHABETICALLY with an `order` field that LOOKS like the runtime value but
-- is not -- a first attempt trusted it (fuel=0, crafter_input=50,
-- crafter_output=51) and every insert failed with "cannot insert to
-- nonexisting inventory". Verified instead against a live game via
-- `factorio-bot rcon -s localhost` while a probe server ran: created a real
-- stone-furnace, called `get_inventory(i)` for i=0..55, and read
-- `defines.inventory.{fuel,crafter_input,crafter_output}` directly --
-- `{fuel=1, crafter_input=2, crafter_output=3}`, matching the non-nil
-- indices {1,2,3,4,6,8} found. A stone-furnace IS a unified "crafter" in
-- Factorio 2.x (no more furnace_source/furnace_result), just not at the
-- indices the doc's `order` field implied.
local FUEL_INV, CRAFTER_INPUT_INV = 1, 2
local furnaces = rcon.find_entities_in_radius(center, 20, "stone-furnace")
print(string.format("SMELT TEST: feeding ore+coal directly into %d furnace(s), belt supply bypassed",
  #furnaces))
rcon.cheat_item(bots[1], "iron-ore", #furnaces * 100 + 10)
rcon.cheat_item(bots[1], "coal", #furnaces * 20 + 10)
local insert_ok, insert_fail = 0, 0
for _, f in ipairs(furnaces) do
  local ok1, err1 = pcall(function()
    rcon.insert_to_inventory(bots[1], "stone-furnace", f.position, CRAFTER_INPUT_INV, "iron-ore", 100)
  end)
  local ok2, err2 = pcall(function()
    rcon.insert_to_inventory(bots[1], "stone-furnace", f.position, FUEL_INV, "coal", 20)
  end)
  if ok1 and ok2 then
    insert_ok = insert_ok + 1
  else
    insert_fail = insert_fail + 1
    if not ok1 then print("  insert iron-ore failed @ " .. tostring(f.position) .. ": " .. tostring(err1)) end
    if not ok2 then print("  insert coal failed @ " .. tostring(f.position) .. ": " .. tostring(err2)) end
  end
end
print(string.format("feed result: %d/%d furnaces fed ok, %d failed", insert_ok, #furnaces, insert_fail))

-- Read one furnace back immediately to confirm the inventory indices landed
-- where intended, rather than assuming the pcall's silence means success.
local function stringify_slots(inv)
  if type(inv) ~= "table" then return "(none)" end
  local parts = {}
  for _, slot in ipairs(inv) do
    if type(slot) == "table" then
      parts[#parts + 1] = tostring(slot.name) .. "x" .. tostring(slot.count)
    end
  end
  if #parts == 0 then return "(empty)" end
  return table.concat(parts, ", ")
end
if #furnaces > 0 then
  local check = rcon.find_entities_in_radius(furnaces[1].position, 0.5, "stone-furnace")
  if check and check[1] then
    print("  furnace[1] after feed: fuel_inventory=" .. stringify_slots(check[1].fuel_inventory)
      .. " output_inventory=" .. stringify_slots(check[1].output_inventory))
  end
end

--- Sum how many of `item` sit in these entities' output inventories.
-- Same idea as scripts/supervisor.lua's `supervisor.count_item`, inlined
-- rather than `include`d to keep this script self-contained.
local function count_output(entities, item)
  local total = 0
  for _, e in ipairs(entities or {}) do
    local inv = e.output_inventory
    if type(inv) == "table" then
      for _, slot in ipairs(inv) do
        if type(slot) == "table" and slot.name == item then
          total = total + (tonumber(slot.count) or 0)
        end
      end
    end
  end
  return total
end

-- Production curve: fixed real-tick checkpoints past the feed, reading each
-- furnace's OWN output_inventory (what it crafted) each time, exactly like
-- supervisor.lua's witness does for one machine -- summed here over all 24.
-- This is direct, in-script evidence; samples.jsonl's own sampling session
-- (started above) independently records force-level item production
-- statistics on its own 300/60-tick beat for `just analyse` to turn into
-- the cumulative/per-minute curve the brief asks for.
print("SMELT TEST: waiting for production, sampling every 600 ticks for 12000 ticks past feed (~3.3 game minutes)")
local feed_tick = rcon.game_tick()
for checkpoint = 1, 20 do
  local target = feed_tick + checkpoint * 600
  while rcon.game_tick() < target do
    -- busy-wait in real (scaled) game time; no os/sleep in the sandbox
  end
  local now_furnaces = rcon.find_entities_in_radius(center, 20, "stone-furnace")
  local plates = count_output(now_furnaces, "iron-plate")
  print(string.format("  tick=%s (+%s past feed) iron-plate in furnace output_inventory=%s",
    tostring(rcon.game_tick()), tostring(checkpoint * 600), tostring(plates)))
end

local id = record.finish(obs.done and "done" or "incomplete")
print("RUN FINISHED id=" .. id)
print("end furnace run")

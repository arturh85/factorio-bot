-- Does `fluidbox_index` cross the bridge? Dumps the world so the recipe table
-- can be read back off disk, which is the same path `factorio-bot plan` uses.
--
-- The answer to check is `recipes["basic-oil-processing"].ingredients[1]
-- .fluidbox_index == 2`: measured on a live 2.1.17 server, that recipe puts
-- crude oil on the refinery's SECOND input box, and a positional rule picks
-- the first one, which the game leaves empty.
print("dumping")
world.dump("fluidbox-probe.json")
print("dumped")

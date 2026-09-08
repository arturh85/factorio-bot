-- Recover the WHOLE production/build history of a loaded save, offline.
--
-- `LuaFlowStatistics.get_flow_count` takes a `sample_index` of 1..300 (1 = most
-- recent), and every precision level holds 300 samples. `ten_hours` is
-- therefore 300 windows of 2 minutes each -- the entire run of any save under
-- 10 hours, at 2-minute resolution, out of the END-STATE save. Nothing else in
-- this project can see a reference run's opening.
--
-- Read-only. Not a host Lua script: this is a `/c` console payload.
--
--   nohup factorio-bot start --settings workspace/wrload.toml -c 0 &
--   P=$(command cat tools/rate_history_probe.lua)
--   factorio-bot rcon --settings workspace/wrload.toml -s localhost -- "/c $P"
--   # lands in <workspace>/server/script-output/wr_rates.csv
--
-- Two things that cost a probe each:
--   * `count = true` is what you want for ITEM statistics (items per window).
--     For ELECTRIC NETWORK statistics it returns a constant that is not
--     energy -- drop it there and read J/tick instead
--     (see tools/rate_history_power_probe.lua).
--   * `get_item_production_statistics` is PER SURFACE. This reads
--     `game.surfaces[1]` only; on the 6:39:53 save that is 99.86% of iron.
--
local f=game.forces.player local s=game.surfaces[1]
local items={"iron-plate","copper-plate","steel-plate","iron-ore","copper-ore","coal","stone","stone-brick","iron-gear-wheel","copper-cable","electronic-circuit","advanced-circuit","processing-unit","plastic-bar","sulfur","automation-science-pack","logistic-science-pack","military-science-pack","chemical-science-pack","production-science-pack","utility-science-pack","space-science-pack","transport-belt","inserter","fast-inserter","assembling-machine-1","assembling-machine-2","assembling-machine-3","electric-mining-drill","burner-mining-drill","stone-furnace","steel-furnace","electric-furnace","lab","boiler","steam-engine","solar-panel","offshore-pump","small-electric-pole","medium-electric-pole","pipe","pumpjack","oil-refinery","chemical-plant","electric-engine-unit","engine-unit","low-density-structure","rocket-fuel","rocket-control-unit","radar","accumulator","big-mining-drill","foundry","electromagnetic-plant"}
local ps={{"ten_hours",defines.flow_precision_index.ten_hours},{"fifty_hours",defines.flow_precision_index.fifty_hours}}
local st=f.get_item_production_statistics(s)
local bs=f.get_entity_build_count_statistics(s)
local out={"kind,name,precision,sample,input,output"}
local function dump(stats,kind,names)
 for _,n in pairs(names) do
  for _,p in pairs(ps) do
   for i=1,300 do
    local ok1,a=pcall(function() return stats.get_flow_count{name=n,category="input",precision_index=p[2],sample_index=i,count=true} end)
    local ok2,b=pcall(function() return stats.get_flow_count{name=n,category="output",precision_index=p[2],sample_index=i,count=true} end)
    if ok1 and ok2 and ((a or 0)~=0 or (b or 0)~=0) then
     out[#out+1]=kind..","..n..","..p[1]..","..i..","..tostring(a)..","..tostring(b)
    end
   end
  end
 end
end
dump(st,"item",items)
local ents={"electric-mining-drill","burner-mining-drill","stone-furnace","steel-furnace","electric-furnace","assembling-machine-1","assembling-machine-2","assembling-machine-3","lab","boiler","steam-engine","solar-panel","offshore-pump","oil-refinery","chemical-plant","pumpjack","transport-belt","inserter","small-electric-pole","medium-electric-pole","big-mining-drill","foundry","electromagnetic-plant","radar","accumulator","beacon"}
dump(bs,"build",ents)
local hdr="#tick="..game.tick..",surfaces="..#game.surfaces..",speed="..game.speed
helpers.write_file("wr_rates.csv",hdr.."\n"..table.concat(out,"\n").."\n",false)
rcon.print("rows="..#out.." tick="..game.tick)

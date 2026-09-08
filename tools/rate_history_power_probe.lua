-- Power half of tools/rate_history_probe.lua: picks the electric network with
-- the largest lifetime production and reads its ten_hours history WITHOUT
-- `count`, i.e. average J/tick per 2-minute window (x60 for watts).
--
local s=game.surfaces[1]
local poles=s.find_entities_filtered{type="electric-pole"}
local best,bestn=nil,-1
for _,p in pairs(poles) do local n=#p.electric_network_statistics.input_counts and 0 or 0 end
local pole=poles[1]
-- pick the pole whose network has the largest total production
local seen={}
for i=1,#poles,50 do
 local p=poles[i]
 local id=p.electric_network_id
 if id and not seen[id] then
  seen[id]=true
  local es=p.electric_network_statistics
  local tot=0
  for n,c in pairs(es.input_counts) do tot=tot+c end
  if tot>bestn then bestn=tot best=p end
 end
end
local es=best.electric_network_statistics
local out={"kind,name,sample,input,output"}
local names={}
for n,_ in pairs(es.input_counts) do names[n]=true end
for n,_ in pairs(es.output_counts) do names[n]=true end
for n,_ in pairs(names) do
 for i=1,300 do
  local ok,a=pcall(function() return es.get_flow_count{name=n,category="input",precision_index=defines.flow_precision_index.ten_hours,sample_index=i} end)
  local ok2,b=pcall(function() return es.get_flow_count{name=n,category="output",precision_index=defines.flow_precision_index.ten_hours,sample_index=i} end)
  if ok and ok2 and ((a or 0)~=0 or (b or 0)~=0) then out[#out+1]="pw,"..n..","..i..","..tostring(a)..","..tostring(b) end
 end
end
helpers.write_file("wr_power2.csv","#tick="..game.tick..",netprod="..bestn..",nets="..tostring(#poles).."\n"..table.concat(out,"\n").."\n",false)
rcon.print("rows="..#out.." netprod="..bestn.." poles="..#poles)

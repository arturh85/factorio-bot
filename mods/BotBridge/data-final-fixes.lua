local maxlen = 200
local rawstr = serpent.dump(data.raw)

local j = 0
for i = 1, string.len(rawstr), maxlen do
	print (i.." / "..string.len(rawstr))
	j = j + 1
	data:extend({{
		type = "virtual-signal",
		name = "DATA_RAW"..j,
		icons = {{icon = "__core__/graphics/empty.png", icon_size = 1}},
		subgroup = "virtual-signal",
		order = string.sub(rawstr,i,i+maxlen-1)
	}});
end
data:extend({{
	type = "virtual-signal",
	name = "DATA_RAW_LEN",
	icons = {{icon = "__core__/graphics/empty.png", icon_size = 1}},
	subgroup = "virtual-signal",
	order = tostring(j)
}});

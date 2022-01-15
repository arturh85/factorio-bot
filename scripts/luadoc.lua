file = io.open("../../docs/src/lua.md", "w")
io.output(file)

io.write("Lua Globals\n")
io.write("===========\n")
io.write("\n")

function dump(tbl, lvl)
    for k,v in pairs(tbl) do
        for i=1,lvl do io.write(" ") end

        if type(v) == "table" then
            io.write("- " .. tostring(k) .. ":\n")
            if lvl < 10 then
                dump(v, lvl + 2)
            end
        else
            io.write("- " .. tostring(k) .. ": " .. tostring(v) .. "\n")
        end
    end
end

for k,v in pairs(_G) do
    io.write(k .. " (" .. type(v) .. ")\n")
    io.write("-----\n")
    io.write("\n")

    if type(v) == "table" then
        dump(v, 0)
    else
        io.write(tostring(v) .. "\n")
        io.write("see LUA Docs\n")
    end

    io.write("\n")
    io.write("\n")
end

io.close(file)
print("done")
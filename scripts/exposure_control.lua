--- The control for the exposure record: a run that is never held.
--
-- Its `exposure.json` must exist and read `holds: []` -- the positive
-- statement "this run was not held", which is what distinguishes it from the
-- absent file that means nobody ever looked.
include("lib.lua")
local run_id = record.start({})
print("no-hold probe: run " .. tostring(run_id))
print("RUN FINISHED state=finished id=" .. record.finish("finished"))

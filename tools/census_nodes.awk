# Stream a factorio-bot world dump's entity_graph.nodes into TSV.
#
#   awk -f tools/census_nodes.awk <dump>.json > nodes.tsv
#
# Columns: name  ore  x  y  direction  lt_x  lt_y  rb_x  rb_y  id
#
# Why not json.load: the world dumps behind the world-record census are ~2.9 GB,
# of which the nodes are the first 1.5%; the rest is edges and the resource
# tree. The dump is pretty-printed with a fixed key order per node, so a line
# state machine reads it in ~2 minutes and never materialises the document.
#
# The collision boxes are shrunk inside their tiles, so floor(lt)..floor(rb) is
# the exact tile footprint -- that, not `position`, is what multi-tile geometry
# has to be computed from.
/"entity_graph": \{/ { ing = 1 }
/"inventories": \[/  { ing = 0 }
ing != 1 { next }
/"left_top": \{/     { lt = NR; next }
/"right_bottom": \{/ { rb = NR; next }
/"position": \{/     { po = NR; next }
NR == lt+1 { ltx = val(); next }
NR == lt+2 { lty = val(); next }
NR == rb+1 { rbx = val(); next }
NR == rb+2 { rby = val(); next }
NR == po+1 { px  = val(); next }
NR == po+2 { py  = val(); next }
/"direction":/   { dir  = str(); next }
/"entity_name":/ { name = str(); next }
/"entity_id":/   { id   = val(); next }
/"miner_ore":/   { ore  = str();
                   print name "\t" ore "\t" px "\t" py "\t" dir "\t" \
                         ltx "\t" lty "\t" rbx "\t" rby "\t" id }
function val(   s) { s = $0; sub(/^[^:]*: */, "", s); sub(/,$/, "", s); return s }
function str(   s) { s = val(); gsub(/"/, "", s); return s }

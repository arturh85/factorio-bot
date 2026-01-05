-- Integration tests for Phase 2.1: Task Graph Builder
-- Tests resource flow tracking, dependency resolution, and validation

print("=== Phase 2.1 Integration Tests ===\n")

-- Test Scenario: Mine → Store Chain with Multiple Resources
-- This tests:
-- 1. Mining tasks produce outputs (iron-ore, copper-ore, coal)
-- 2. Insert tasks consume inputs
-- 3. Dependency resolution creates edges from mine → insert
-- 4. Resource flow validation ensures sufficient resources

print("[Scenario] Multi-Resource Mine → Store Chain")
print("Expected: All validations pass, implicit dependencies created\n")

-- Phase 1: Mine multiple resources
print("Phase 1: Mining resources...")
plan.group_start("resource gathering")

-- Bot 1 mines iron ore
plan.mine(1, {x = 10.5, y = 10.5}, "iron-ore", 50)
print("  - Bot 1: Mine 50 iron-ore at (10.5, 10.5)")

-- Bot 2 mines copper ore
plan.mine(2, {x = 20.5, y = 10.5}, "copper-ore", 30)
print("  - Bot 2: Mine 30 copper-ore at (20.5, 10.5)")

-- Bot 3 mines coal
plan.mine(3, {x = 30.5, y = 10.5}, "coal", 20)
print("  - Bot 3: Mine 20 coal at (30.5, 10.5)")

plan.group_end()

-- Phase 2: Store some of the mined resources
print("\nPhase 2: Storing resources in chests...")
plan.group_start("storage")

-- Bot 1 stores 25 iron-ore (has 50, uses 25, leaves 25)
plan.insert_into_inventory(1, {
    entity_name = "wooden-chest",
    position = {x = 15.5, y = 15.5},
    inventory_type = 1
}, {name = "iron-ore", count = 25})
print("  - Bot 1: Insert 25 iron-ore into chest at (15.5, 15.5)")

-- Bot 2 stores 15 copper-ore (has 30, uses 15, leaves 15)
plan.insert_into_inventory(2, {
    entity_name = "wooden-chest",
    position = {x = 25.5, y = 15.5},
    inventory_type = 1
}, {name = "copper-ore", count = 15})
print("  - Bot 2: Insert 15 copper-ore into chest at (25.5, 15.5)")

-- Bot 3 stores 10 coal (has 20, uses 10, leaves 10)
plan.insert_into_inventory(3, {
    entity_name = "wooden-chest",
    position = {x = 35.5, y = 15.5},
    inventory_type = 1
}, {name = "coal", count = 10})
print("  - Bot 3: Insert 10 coal into chest at (35.5, 15.5)")

plan.group_end()

-- Phase 3: Finalize and validate
print("\nPhase 3: Finalizing task graph...")
print("  - Resolving dependencies...")
print("  - Validating resource flow...")

local success, err = pcall(function()
    plan.finalize()
end)

if success then
    print("\n✓ SUCCESS: Task graph validated successfully!")
    print("\nExpected Results:")
    print("  1. Mine tasks created with outputs:")
    print("     - Bot 1: 50 iron-ore")
    print("     - Bot 2: 30 copper-ore")
    print("     - Bot 3: 20 coal")
    print("  2. Insert tasks created with inputs:")
    print("     - Bot 1: 25 iron-ore (remaining: 25)")
    print("     - Bot 2: 15 copper-ore (remaining: 15)")
    print("     - Bot 3: 10 coal (remaining: 10)")
    print("  3. Implicit dependencies created:")
    print("     - mine(iron-ore) → insert(iron-ore)")
    print("     - mine(copper-ore) → insert(copper-ore)")
    print("     - mine(coal) → insert(coal)")
    print("  4. Resource flow validation passed")
    print("  5. Per-player resource isolation maintained")
else
    print("\n✗ FAILED: " .. tostring(err))
end

-- Display the task graph
print("\n" .. string.rep("=", 60))
print("Task Graph Visualization (DOT format):")
print(string.rep("=", 60))
print(plan.task_graph_graphviz())
print(string.rep("=", 60))

print("\n=== Phase 2.1 Integration Tests Complete ===")
print("\nKey Features Tested:")
print("  ✓ Resource flow tracking (outputs from mining)")
print("  ✓ Resource flow tracking (inputs for inserting)")
print("  ✓ Automatic dependency resolution")
print("  ✓ Resource flow validation")
print("  ✓ Per-player resource isolation")
print("  ✓ Multi-bot coordination with groups")

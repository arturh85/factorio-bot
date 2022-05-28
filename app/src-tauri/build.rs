use factorio_bot_core::settings::*;
use factorio_bot_core::types::*;

#[cfg(feature = "rest")]
use factorio_bot_restapi::settings::RestApiSettings;
use std::fs;
use typescript_definitions::TypeScriptifyTrait;

const TYPESCRIPT_SETTINGS_PATH: &str = "../src/models/types.ts";

fn main() {
  typescriptify();
  #[cfg(feature = "gui")]
  {
    tauri_build::build();
  }
}

fn typescriptify() {
  let existing =
    std::fs::read_to_string(TYPESCRIPT_SETTINGS_PATH).expect("types.ts does not exist?");
  let lines: Vec<&str> = existing.split('\n').collect();
  let autogenerated_marker = lines
    .iter()
    .position(|line| line.contains("AUTOGENERATED"))
    .expect("AUTOGENERATED marker not found");

  let mut output = String::from(&lines[0..autogenerated_marker + 1].join("\n")) + "\n";
  output += &FactorioFluidBoxPrototype::type_script_ify();
  output += &FactorioFluidBoxConnection::type_script_ify();
  output += &FactorioBlueprintInfo::type_script_ify();
  output += &PlayerChangedDistanceEvent::type_script_ify();
  output += &PlayerChangedPositionEvent::type_script_ify();
  output += &PlayerChangedMainInventoryEvent::type_script_ify();
  output += &PlayerLeftEvent::type_script_ify();
  output += &RequestEntity::type_script_ify();
  output += &FactorioTile::type_script_ify();
  output += &FactorioTechnology::type_script_ify();
  output += &FactorioForce::type_script_ify();
  output += &InventoryResponse::type_script_ify();
  output += &FactorioRecipe::type_script_ify();
  output += &PlaceEntityResult::type_script_ify();
  output += &PlaceEntitiesResult::type_script_ify();
  output += &FactorioIngredient::type_script_ify();
  output += &FactorioProduct::type_script_ify();
  output += &FactorioPlayer::type_script_ify();
  output += &ChunkPosition::type_script_ify();
  output += &Position::type_script_ify();
  output += &Rect::type_script_ify();
  output += &FactorioChunk::type_script_ify();
  output += &ChunkObject::type_script_ify();
  output += &ChunkResource::type_script_ify();
  output += &FactorioGraphic::type_script_ify();
  output += &FactorioEntity::type_script_ify();
  output += &FactorioEntityPrototype::type_script_ify();
  output += &FactorioItemPrototype::type_script_ify();
  output += &FactorioResult::type_script_ify();
  output += &PrimeVueTreeNode::type_script_ify();
  output += &FactorioSettings::type_script_ify();
  if cfg!(feature = "rest") {
    output += &RestApiSettings::type_script_ify();
  }

  output = output.replace("DateTime<Utc>", "String");
  output = output.replace("DateTime<    Utc>", "String");
  output = output.replace("DateTime    <Utc>", "String");
  output = output.replace("NaiveDate", "String");
  output = output.replace("R64", "number");
  output = output.replace(": Value", ": object");
  output = output.replace("};", "};\n");
  while output.contains("  ") {
    output = output.replace("  ", " ");
  }
  output = output.replace("\n\n", "\n");
  fs::write(TYPESCRIPT_SETTINGS_PATH, output).expect("failed to write typescript types");
}

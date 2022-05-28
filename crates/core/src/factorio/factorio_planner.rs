use std::io::Read;

use base64::decode;
use flate2::read::ZlibDecoder;
use miette::{IntoDiagnostic, Result};
use serde_json::Value;

pub fn decode_zlibencoded_json(exchange_str: &str) -> Result<serde_json::Value> {
    let content = decode(exchange_str).into_diagnostic()?;
    let mut content = ZlibDecoder::new(&content[..]);
    let mut s = String::new();
    content.read_to_string(&mut s).unwrap();
    let v: Value = serde_json::from_str(&s).into_diagnostic()?;
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parsing() {
        let input = "eNrtV01v1EAM/Stozkm16aGHvSKBkKiE6BGhaOI4WZf5CJ5J0WqV/44nH1q621LRAkIix9hv7Of34nwclPV1eYccyDu1VcXF5uJKZSr0VaMheiYMavvpoJy2KPm3jOhe3QChAxQcQTp2UHHfpTRFtBKdwca3FCJBHiZ83mn4ooZMRbIYQBsBXW0E7mPqouTkB/Z1DzGV9NUtQpy6d+yjT8GfVhbaZDtDDWGttpF7zO4Rk86MX3tirEttfe/GPjU25CRS7QVXoZGWmVrSRTaGylMCkbULneeYp/QDnYc0pu9Kg3doFjJgdEiDLlMOn7N5zHJJvZsUXC5fe2MkncyZCzbGe05M3gvrU52WY2MuTQvUjaDnCwg6Yus5iQOsm0iuTVS6aQQhVs4STxH8gfzHqX1CIwO6qFsBFhux3GrYzfxPqclZtJWRPvmMyotfYbbTXJeGLIl9jTZBkNfCzNwX6zCcST+jHhPfjukSphtjc4Rdz7OI5WcOpD36LYY9ecOtTr3MqcelJ/Yub1Fz/m2Hssyr9i/T/vyhN1vwcMX5CXp5zL8ZN+qPrhu5gByRV7PXRfsvtMdUWvSXTwIghp7Wd8zflB98J6rkoKux5yr8P/GCeV6h4qzQMXCz/Fft5eN/+A46FKN5";
        assert!(decode_zlibencoded_json(input).is_ok());
    }
}

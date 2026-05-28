use std::str;

pub fn is_valid_uuid_v4(s: &str) -> bool {
    // Check if the string has the correct length for a UUID v4
    if s.len() != 36 {
        return false;
    }
    
    // Split the string by hyphens
    let parts: Vec<&str> = s.split('-').collect();
    
    // Check if we have exactly 5 parts
    if parts.len() != 5 {
        return false;
    }
    
    // Check each part has the correct length
    let expected_lengths = [8, 4, 4, 4, 12];
    for (i, &length) in expected_lengths.iter().enumerate() {
        if parts[i].len() != length {
            return false;
        }
    }
    
    // Check that all characters are valid hexadecimal digits
    for part in &parts {
        for c in part.chars() {
            if !c.is_ascii_hexdigit() {
                return false;
            }
        }
    }
    
    // Additional check: the 13th character (index 12) should be '4' for UUID v4
    if !parts[2].starts_with('4') {
        return false;
    }
    
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_uuid_v4() {
        assert!(is_valid_uuid_v4("550e8400-e29b-41d4-a716-446655440000"));
        assert!(is_valid_uuid_v4("12345678-1234-4567-8901-234567890123"));
        assert!(is_valid_uuid_v4("00000000-0000-4000-8000-000000000000"));
    }

    #[test]
    fn test_invalid_uuid_v4() {
        assert!(!is_valid_uuid_v4("550e8400-e29b-41d4-a716-44665544000")); // too short
        assert!(!is_valid_uuid_v4("550e8400-e29b-41d4-a716-4466554400000")); // too long
        assert!(!is_valid_uuid_v4("550e8400-e29b-41d4-a716-44665544000g")); // invalid char
        assert!(!is_valid_uuid_v4("550e8400-e29b-11d4-a716-446655440000")); // wrong version
        assert!(!is_valid_uuid_v4("550e8400-e29b-41d4-a716-44665544000")); // missing hyphen
        assert!(!is_valid_uuid_v4("")); // empty string
    }
}
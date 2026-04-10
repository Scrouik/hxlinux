// Identifiants USB du HX Stomp XL
pub const VENDOR_ID: u16 = 0x0e41;
pub const PRODUCT_ID: u16 = 0x4253;

// Endpoints USB
pub const ENDPOINT_BULK_OUT: u8 = 0x01;
pub const ENDPOINT_BULK_IN: u8 = 0x81;

// Protocole
pub const PRESET_COUNT: usize = 125;
pub const PRESET_NAME_PATTERN: [u8; 3] = [0x81, 0xcd, 0x00];
pub const PRESET_NAME_LENGTH: usize = 16;

// Message de connexion initial
pub const CONNECT_INIT: [u8; 20] = [
    0x0c, 0x00, 0x00, 0x28,
    0x01, 0x10, 0xef, 0x03,
    0x00, 0x00, 0x00, 0x02,
    0x00, 0x01, 0x00, 0x21,
    0x00, 0x10, 0x00, 0x00
];

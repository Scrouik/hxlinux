use rusb::{Context, DeviceHandle, UsbContext};
use crate::helix::protocol::*;

pub struct HelixUsb {
    handle: DeviceHandle<Context>,
}

impl HelixUsb {
    pub fn connect() -> Result<Self, rusb::Error> {
        let context = Context::new()?;
        
        let handle = context
            .open_device_with_vid_pid(VENDOR_ID, PRODUCT_ID)
            .ok_or(rusb::Error::NoDevice)?;

        Ok(HelixUsb { handle })
    }

    pub fn write(&self, data: &[u8]) -> Result<(), rusb::Error> {
        let timeout = std::time::Duration::from_millis(1000);
        self.handle.write_bulk(ENDPOINT_BULK_OUT, data, timeout)?;
        Ok(())
    }

    pub fn read(&self, buf: &mut [u8]) -> Result<usize, rusb::Error> {
        let timeout = std::time::Duration::from_millis(1000);
        let n = self.handle.read_bulk(ENDPOINT_BULK_IN, buf, timeout)?;
        Ok(n)
    }
}
use crate::types::{DeviceDescriptor, Viewport};

/// iPhone 14 device descriptor.
pub fn iphone_14() -> DeviceDescriptor {
    DeviceDescriptor {
        name: "iPhone 14",
        user_agent: "Mozilla/5.0 (iPhone; CPU iPhone OS 16_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/16.0 Mobile/15E148 Safari/604.1".into(),
        viewport: Viewport {
            width: 390,
            height: 844,
            device_scale_factor: Some(3.0),
            is_mobile: Some(true),
            has_touch: Some(true),
            is_landscape: Some(false),
        },
    }
}

/// iPhone 14 in landscape mode.
pub fn iphone_14_landscape() -> DeviceDescriptor {
    DeviceDescriptor {
        name: "iPhone 14 landscape",
        user_agent: "Mozilla/5.0 (iPhone; CPU iPhone OS 16_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/16.0 Mobile/15E148 Safari/604.1".into(),
        viewport: Viewport {
            width: 844,
            height: 390,
            device_scale_factor: Some(3.0),
            is_mobile: Some(true),
            has_touch: Some(true),
            is_landscape: Some(true),
        },
    }
}

/// iPad (10th generation) device descriptor.
pub fn ipad() -> DeviceDescriptor {
    DeviceDescriptor {
        name: "iPad",
        user_agent: "Mozilla/5.0 (iPad; CPU OS 16_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/16.0 Mobile/15E148 Safari/604.1".into(),
        viewport: Viewport {
            width: 820,
            height: 1180,
            device_scale_factor: Some(2.0),
            is_mobile: Some(true),
            has_touch: Some(true),
            is_landscape: Some(false),
        },
    }
}

/// Pixel 5 device descriptor.
pub fn pixel_5() -> DeviceDescriptor {
    DeviceDescriptor {
        name: "Pixel 5",
        user_agent: "Mozilla/5.0 (Linux; Android 11; Pixel 5) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/99.0.4844.58 Mobile Safari/537.36".into(),
        viewport: Viewport {
            width: 393,
            height: 851,
            device_scale_factor: Some(2.75),
            is_mobile: Some(true),
            has_touch: Some(true),
            is_landscape: Some(false),
        },
    }
}

/// Desktop 1920×1080 device descriptor.
pub fn desktop_1080p() -> DeviceDescriptor {
    DeviceDescriptor {
        name: "Desktop 1080p",
        user_agent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/99.0.4844.51 Safari/537.36".into(),
        viewport: Viewport {
            width: 1920,
            height: 1080,
            device_scale_factor: Some(1.0),
            is_mobile: Some(false),
            has_touch: Some(false),
            is_landscape: Some(false),
        },
    }
}

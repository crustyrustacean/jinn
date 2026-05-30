pub struct RgbColor {
    red: u8,
    green: u8,
    blue: u8,
    alpha: u8,
    components: Vec<u8>,
}

impl RgbColor {
    pub fn new(red: u8, green: u8, blue: u8) -> Self {
        Self {
            red,
            green,
            blue,
            alpha: 255,
            components: vec![red, green, blue],
        }
    }

    pub fn to_hex(&self) -> String {
        format!("{:02x}{:02x}{:02x}", self.red, self.green, self.blue)
    }
}

pub struct HslColor {
    hue: f32,
    saturation: f32,
    lightness: f32,
    alpha: f32,
    components: Vec<f32>,
}

impl HslColor {
    pub fn new(hue: f32, saturation: f32, lightness: f32) -> Self {
        Self {
            hue,
            saturation,
            lightness,
            alpha: 1.0,
            components: vec![hue, saturation, lightness],
        }
    }

    pub fn to_hex(&self) -> String {
        format!("{:02x}{:02x}{:02x}", self.hue as u8, self.saturation as u8, self.lightness as u8)
    }
}

pub struct CmykColor {
    cyan: f32,
    magenta: f32,
    yellow: f32,
    key: f32,
    components: Vec<f32>,
}

impl CmykColor {
    pub fn new(cyan: f32, magenta: f32, yellow: f32, key: f32) -> Self {
        Self {
            cyan,
            magenta,
            yellow,
            key,
            components: vec![cyan, magenta, yellow, key],
        }
    }

    pub fn to_hex(&self) -> String {
        format!("{:02x}{:02x}{:02x}", self.cyan as u8, self.magenta as u8, self.yellow as u8)
    }
}

pub struct HsvColor {
    hue: f32,
    saturation: f32,
    value: f32,
    alpha: f32,
    components: Vec<f32>,
}

impl HsvColor {
    pub fn new(hue: f32, saturation: f32, value: f32) -> Self {
        Self {
            hue,
            saturation,
            value,
            alpha: 1.0,
            components: vec![hue, saturation, value],
        }
    }

    pub fn to_hex(&self) -> String {
        format!("{:02x}{:02x}{:02x}", self.hue as u8, self.saturation as u8, self.value as u8)
    }
}

pub struct YuvColor {
    y: f32,
    u: f32,
    v: f32,
    alpha: f32,
    components: Vec<f32>,
}

impl YuvColor {
    pub fn new(y: f32, u: f32, v: f32) -> Self {
        Self {
            y,
            u,
            v,
            alpha: 1.0,
            components: vec![y, u, v],
        }
    }

    pub fn to_hex(&self) -> String {
        format!("{:02x}{:02x}{:02x}", self.y as u8, self.u as u8, self.v as u8)
    }
}

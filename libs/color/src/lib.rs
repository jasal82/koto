//! A Koto language module for working with colors

mod color;

pub use color::Color;

use koto_runtime::{prelude::*, Result};
use palette::{Hsl, Hsv};

pub fn make_module() -> KMap {
    use Value::{Number, Str};
    let mut result = KMap::default();

    result.add_fn("hsl", |ctx| match ctx.args() {
        [Number(h), Number(s), Number(l)] => {
            let hsv = Hsl::new(f32::from(h), f32::from(s), f32::from(l));
            Ok(Color::from(hsv).into())
        }
        unexpected => type_error_with_slice("3 Numbers, with hue specified in degrees", unexpected),
    });

    result.add_fn("hsv", |ctx| match ctx.args() {
        [Number(h), Number(s), Number(v)] => {
            let hsv = Hsv::new(f32::from(h), f32::from(s), f32::from(v));
            Ok(Color::from(hsv).into())
        }
        unexpected => type_error_with_slice("3 Numbers, with hue specified in degrees", unexpected),
    });

    result.add_fn("named", |ctx| match ctx.args() {
        [Str(s)] => named(s),
        unexpected => type_error_with_slice("a String", unexpected),
    });

    result.add_fn("rgb", |ctx| match ctx.args() {
        [Number(r), Number(g), Number(b)] => rgb(r, g, b),
        unexpected => type_error_with_slice("3 Numbers", unexpected),
    });

    result.add_fn("rgba", |ctx| match ctx.args() {
        [Number(r), Number(g), Number(b), Number(a)] => rgba(r, g, b, a),
        unexpected => type_error_with_slice("4 Numbers", unexpected),
    });

    let mut meta = MetaMap::default();

    meta.insert(MetaKey::Type, "color".into());
    meta.add_fn(MetaKey::Call, |ctx| match ctx.args() {
        [Str(s)] => named(s),
        [Number(r), Number(g), Number(b)] => rgb(r, g, b),
        [Number(r), Number(g), Number(b), Number(a)] => rgba(r, g, b, a),
        unexpected => type_error_with_slice("a String", unexpected),
    });

    result.set_meta_map(Some(meta));
    result
}

fn named(name: &str) -> Result<Value> {
    match Color::named(name) {
        Some(c) => Ok(c.into()),
        None => Ok(Value::Null),
    }
}

fn rgb(r: &KNumber, g: &KNumber, b: &KNumber) -> Result<Value> {
    Ok(Color::rgb(r.into(), g.into(), b.into()).into())
}

fn rgba(r: &KNumber, g: &KNumber, b: &KNumber, a: &KNumber) -> Result<Value> {
    Ok(Color::rgba(r.into(), g.into(), b.into(), a.into()).into())
}

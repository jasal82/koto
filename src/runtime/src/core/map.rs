use crate::{external_error, value_is_immutable, Value, ValueList, ValueMap, ValueVec};

pub fn make_module() -> ValueMap {
    use Value::*;

    let mut result = ValueMap::new();

    result.add_fn("contains_key", |_, args| match args {
        [Map(m), key] => Ok(Bool(m.data().contains_key(key))),
        _ => external_error!("map.contains_key: Expected map and key as arguments"),
    });

    result.add_fn("keys", |_, args| match args {
        [Map(m)] => Ok(List(ValueList::with_data(
            m.data().keys().cloned().collect::<ValueVec>(),
        ))),
        _ => external_error!("map.keys: Expected map as argument"),
    });

    result.add_fn("get", |_, args| match args {
        [Map(m), key] => match m.data().get(key) {
            Some(value) => Ok(value.clone()),
            None => Ok(Empty),
        },
        _ => external_error!("map.get: Expected map and key as arguments"),
    });

    result.add_fn("insert", |_, args| match args {
        [Map(m), key] if value_is_immutable(key) => match m.data_mut().insert(key.clone(), Empty) {
            Some(old_value) => Ok(old_value),
            None => Ok(Empty),
        },
        [Map(m), key, value] if value_is_immutable(key) => {
            match m.data_mut().insert(key.clone(), value.clone()) {
                Some(old_value) => Ok(old_value),
                None => Ok(Empty),
            }
        }
        _ => external_error!("map.insert: Expected map and key as arguments"),
    });

    result.add_fn("is_empty", |_, args| match args {
        [Map(m)] => Ok(Bool(m.data().is_empty())),
        _ => external_error!("map.contains_key: Expected map and key as arguments"),
    });

    result.add_fn("remove", |_, args| match args {
        [Map(m), key] if value_is_immutable(key) => match m.data_mut().remove(key) {
            Some(old_value) => Ok(old_value),
            None => Ok(Empty),
        },
        _ => external_error!("map.remove: Expected map and key as arguments"),
    });

    result.add_fn("size", |_, args| match args {
        [Map(m)] => Ok(Number(m.data().len() as f64)),
        _ => external_error!("map.contains_key: Expected map and key as arguments"),
    });

    result.add_fn("values", |_, args| match args {
        [Map(m)] => Ok(List(ValueList::with_data(
            m.data().values().cloned().collect::<ValueVec>(),
        ))),
        _ => external_error!("map.keys: Expected map as argument"),
    });

    result
}

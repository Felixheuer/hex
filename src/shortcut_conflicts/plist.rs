//! Preferences as JSON values: another app's or macOS's saved defaults
//! domain, read through `NSUserDefaults`, so the checks above parse plain
//! values (and tests feed them fixture plists).

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_foundation::{NSArray, NSDictionary, NSNumber, NSString, NSUserDefaults};
use serde_json::{Map, Value};

/// The saved defaults domain `name` (such as `com.apple.symbolichotkeys`),
/// or `Null` when it has none.
pub(super) fn domain(name: &str) -> Value {
    NSUserDefaults::standardUserDefaults()
        .persistentDomainForName(&NSString::from_str(name))
        .map_or(Value::Null, |domain| {
            let domain: &AnyObject = &domain;
            value(domain)
        })
}

/// A property-list object as JSON: dictionaries with string keys, arrays,
/// numbers (booleans as 0 or 1), and strings; anything else is `Null`.
pub(super) fn value(object: &AnyObject) -> Value {
    if let Some(number) = object.downcast_ref::<NSNumber>() {
        let whole = number.longLongValue();
        let real = number.doubleValue();
        return if whole as f64 == real {
            Value::from(whole)
        } else {
            Value::from(real)
        };
    }
    if let Some(text) = object.downcast_ref::<NSString>() {
        return Value::String(text.to_string());
    }
    if let Some(array) = object.downcast_ref::<NSArray>() {
        return Value::Array(
            (0..array.count())
                .map(|index| value(&array.objectAtIndex(index)))
                .collect(),
        );
    }
    if let Some(dictionary) = object.downcast_ref::<NSDictionary>() {
        let keys = dictionary.allKeys();
        let mut map = Map::new();
        for index in 0..keys.count() {
            let key: Retained<AnyObject> = keys.objectAtIndex(index);
            let Some(name) = key.downcast_ref::<NSString>() else {
                continue;
            };
            if let Some(entry) = dictionary.objectForKey(&key) {
                map.insert(name.to_string(), value(&entry));
            }
        }
        return Value::Object(map);
    }
    Value::Null
}

/// A plist document (XML or binary) as JSON, for fixtures.
#[cfg(test)]
pub(super) fn parse(document: &str) -> Value {
    use objc2_foundation::{NSData, NSPropertyListMutabilityOptions, NSPropertyListSerialization};
    let data = NSData::with_bytes(document.as_bytes());
    // SAFETY: a null format pointer is documented as "don't report it".
    let object = unsafe {
        NSPropertyListSerialization::propertyListWithData_options_format_error(
            &data,
            NSPropertyListMutabilityOptions::Immutable,
            std::ptr::null_mut(),
        )
    }
    .expect("a valid fixture plist");
    value(&object)
}

/// `value[key]` as an integer.
pub(super) fn integer(value: &Value, key: &str) -> Option<i64> {
    value.get(key)?.as_i64()
}

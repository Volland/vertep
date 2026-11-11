//! Conversion between boa values and the datamodel-independent [`Value`].

use boa_engine::value::JsVariant;
use boa_engine::{
    Context, JsNativeError, JsObject, JsResult, JsString, JsValue, js_string,
    object::builtins::JsArray,
};
use vertep_core::datamodel::Value;

/// Deepest nesting converted before giving up, to bound the work on pathological values.
const MAX_DEPTH: usize = 512;

/// A [`Value`] as an ECMAScript value: arrays become arrays, objects become plain objects.
pub(crate) fn to_js(value: &Value, ctx: &mut Context) -> JsResult<JsValue> {
    Ok(match value {
        Value::Undefined => JsValue::undefined(),
        Value::Null => JsValue::null(),
        Value::Bool(b) => JsValue::new(*b),
        Value::Number(n) => JsValue::rational(*n),
        Value::String(s) => JsValue::new(JsString::from(s.as_str())),
        Value::Array(items) => {
            let mut values = Vec::with_capacity(items.len());
            for item in items {
                values.push(to_js(item, ctx)?);
            }
            JsArray::from_iter(values, ctx).into()
        }
        Value::Object(entries) => {
            let object = JsObject::with_object_proto(ctx.intrinsics());
            for (key, item) in entries {
                let item = to_js(item, ctx)?;
                object.create_data_property_or_throw(JsString::from(key.as_str()), item, ctx)?;
            }
            object.into()
        }
    })
}

/// An ECMAScript value as a [`Value`], following `JSON.stringify` where JSON has no answer:
/// functions become `undefined` (and are left out of objects), `toJSON` is honored (so a
/// `Date` becomes its ISO string), BigInts become numbers, and symbols and cycles are errors.
/// `undefined` is kept, in arrays and in objects.
pub(crate) fn from_js(
    value: &JsValue,
    keys: &JsObject,
    is_array: &JsObject,
    ctx: &mut Context,
) -> JsResult<Value> {
    let mut stack = Vec::new();
    convert(value, keys, is_array, ctx, &mut stack)
}

fn convert(
    value: &JsValue,
    keys: &JsObject,
    is_array: &JsObject,
    ctx: &mut Context,
    stack: &mut Vec<JsObject>,
) -> JsResult<Value> {
    let object = match value.variant() {
        JsVariant::Undefined => return Ok(Value::Undefined),
        JsVariant::Null => return Ok(Value::Null),
        JsVariant::Boolean(b) => return Ok(Value::Bool(b)),
        JsVariant::String(s) => return Ok(Value::String(s.to_std_string_escaped())),
        JsVariant::Float64(n) => return Ok(Value::Number(n)),
        JsVariant::Integer32(n) => return Ok(Value::Number(f64::from(n))),
        JsVariant::BigInt(n) => return Ok(Value::Number(n.to_f64())),
        JsVariant::Symbol(_) => {
            return Err(JsNativeError::typ()
                .with_message("a symbol cannot leave the datamodel")
                .into());
        }
        JsVariant::Object(object) => object,
    };
    if object.is_callable() {
        return Ok(Value::Undefined);
    }
    if stack.iter().any(|seen| JsObject::equals(seen, &object)) {
        return Err(JsNativeError::typ()
            .with_message("a cyclic value cannot leave the datamodel")
            .into());
    }
    if stack.len() >= MAX_DEPTH {
        return Err(JsNativeError::range()
            .with_message("value is nested too deeply to leave the datamodel")
            .into());
    }
    let to_json = object.get(js_string!("toJSON"), ctx)?;
    if let Some(to_json) = to_json.as_callable() {
        let replaced = to_json.call(&object.clone().into(), &[], ctx)?;
        let same = replaced
            .as_object()
            .is_some_and(|r| JsObject::equals(&r, &object));
        if !same {
            stack.push(object);
            let result = convert(&replaced, keys, is_array, ctx, stack);
            stack.pop();
            return result;
        }
    }
    stack.push(object.clone());
    let result = convert_object(&object, keys, is_array, ctx, stack);
    stack.pop();
    result
}

fn convert_object(
    object: &JsObject,
    keys: &JsObject,
    is_array: &JsObject,
    ctx: &mut Context,
    stack: &mut Vec<JsObject>,
) -> JsResult<Value> {
    let this = JsValue::undefined();
    let object_value: JsValue = object.clone().into();
    if is_array
        .call(&this, std::slice::from_ref(&object_value), ctx)?
        .to_boolean()
    {
        let length = object.get(js_string!("length"), ctx)?.to_length(ctx)?;
        let length = u32::try_from(length).unwrap_or(u32::MAX);
        let mut items = Vec::new();
        for index in 0..length {
            let item = object.get(index, ctx)?;
            items.push(convert(&item, keys, is_array, ctx, stack)?);
        }
        return Ok(Value::Array(items));
    }
    let names = keys.call(&this, &[object_value], ctx)?;
    let names = names
        .as_object()
        .ok_or_else(|| JsNativeError::typ().with_message("Object.keys returned a non-object"))?;
    let count = names.get(js_string!("length"), ctx)?.to_length(ctx)?;
    let count = u32::try_from(count).unwrap_or(u32::MAX);
    let mut entries = Vec::new();
    for index in 0..count {
        let name = names.get(index, ctx)?.to_string(ctx)?;
        let item = object.get(name.clone(), ctx)?;
        if item.is_callable() {
            continue;
        }
        entries.push((
            name.to_std_string_escaped(),
            convert(&item, keys, is_array, ctx, stack)?,
        ));
    }
    Ok(Value::Object(entries))
}

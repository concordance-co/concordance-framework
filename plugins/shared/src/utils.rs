#[macro_export]
macro_rules! inlined_schema_for {
    ($type:ty) => {
        schemars::generate::SchemaSettings::default()
            .for_serialize()
            .with(|s| s.meta_schema = None)
            .with(|s| s.inline_subschemas = true)
            .with_transform(schemars::transform::ReplaceConstValue::default())
            .with_transform(schemars::transform::RecursiveTransform(
                |schema: &mut schemars::Schema| {
                    if schema.get("properties").is_some() {
                        schema.insert("additionalProperties".to_owned(), false.into());
                        schema.insert("strict".to_owned(), true.into());
                    }

                    if schema.get("parameters").is_some() {
                        // schema.insert("strict".to_owned(), true.into());
                    }

                    // cant have ref and description
                    if schema.get("$ref").is_some() && schema.get("description").is_some() {
                        schema.remove("description");
                    }

                    if let Some(one_of) = schema.remove("oneOf") {
                        schema.insert("anyOf".to_owned(), one_of);
                    }

                    if let Some(items) = schema.remove("items") {
                        if items.as_bool().is_some() {
                            // any type
                            let new_items = serde_json::json!({
                                "anyOf": [
                                    {"type": "string"},
                                    {"type": "number"},
                                    {"type": "boolean"},
                                    {"type": "array",
                                        "items": {
                                            "type": ["string", "number", "boolean"]
                                        },
                                        "additionalProperties": false,
                                    }
                                ]});
                            schema.insert("items".to_owned(), new_items);
                        } else {
                            schema.insert("items".to_owned(), items);
                        }
                    }

                    if let Some(prefix) = schema.remove("prefixItems") {
                        schema.remove("maxItems");
                        schema.remove("minItems");
                        let fixed_items = prefix.as_array().unwrap();
                        let mut types = Vec::new();
                        fixed_items.into_iter().for_each(|item| {
                            types.push(
                                item.as_object()
                                    .unwrap()
                                    .get("type")
                                    .unwrap()
                                    .as_str()
                                    .unwrap()
                                    .to_owned(),
                            );
                        });
                        types.sort();
                        types.dedup();

                        schema.insert("items".to_owned(), serde_json::json!({ "type": types}));
                    }
                },
            ))
            .into_generator()
            .into_root_schema_for::<$type>()
    };
}

#[macro_export]
macro_rules! with_examples_inlined_schema_for {
    ($type:ty) => {
        with_examples_inlined_schema_for!($type, $type::default())
    };
    ($type:ty, $($examples:expr),*) => {
        schemars::generate::SchemaSettings::default()
            .for_serialize()
            .with(|s| s.meta_schema = None)
            .with(|s| s.inline_subschemas = true)
            .with_transform(schemars::transform::ReplaceConstValue::default())
            .with_transform(schemars::transform::RecursiveTransform(
                |schema: &mut schemars::Schema| {
                    if schema.get("title").is_some() {
                        schema.insert("examples".to_owned(), serde_json::to_value(&vec![$($examples),*]).unwrap());
                    }
                    if schema.get("properties").is_some() {
                        schema.insert("additionalProperties".to_owned(), false.into());
                        schema.insert("strict".to_owned(), true.into());
                    }

                    if schema.get("parameters").is_some() {
                        // schema.insert("strict".to_owned(), true.into());
                    }

                    // cant have ref and description
                    if schema.get("$ref").is_some() && schema.get("description").is_some() {
                        schema.remove("description");
                    }

                    if let Some(one_of) = schema.remove("oneOf") {
                        schema.insert("anyOf".to_owned(), one_of);
                    }

                    if let Some(items) = schema.remove("items") {
                        if items.as_bool().is_some() {
                            // array of arbitrary json -
                            let new_items = serde_json::json!({
                                "anyOf": [
                                    {"type": "string"},
                                    {"type": "number"},
                                    {"type": "boolean"},
                                    {"type": "array",
                                        "items": {
                                            "type": ["string", "number", "boolean"]
                                        },
                                        "additionalProperties": false,
                                    }
                                ]});
                            schema.insert("items".to_owned(), new_items);
                        } else {
                            schema.insert("items".to_owned(), items);
                        }
                    }

                    if let Some(prefix) = schema.remove("prefixItems") {
                        schema.remove("maxItems");
                        schema.remove("minItems");
                        let fixed_items = prefix.as_array().unwrap();
                        let mut types = Vec::new();
                        fixed_items.into_iter().for_each(|item| {
                            types.push(
                                item.as_object()
                                    .unwrap()
                                    .get("type")
                                    .unwrap()
                                    .as_str()
                                    .unwrap()
                                    .to_owned(),
                            );
                        });
                        types.sort();
                        types.dedup();

                        schema.insert("items".to_owned(), serde_json::json!({ "type": types}));
                    }
                },
            ))
            .into_generator()
            .into_root_schema_for::<$type>()
    };
}

#[macro_export]
macro_rules! response_format_inlined_schema_for {
    ($type:ty) => {
        schemars::generate::SchemaSettings::default()
            .for_serialize()
            .with(|s| s.meta_schema = None)
            .with(|s| s.inline_subschemas = true)
            .with_transform(schemars::transform::ReplaceConstValue::default())
            .with_transform(schemars::transform::RecursiveTransform(
                |schema: &mut schemars::Schema| {
                    // https://platform.openai.com/docs/guides/structured-outputs?api-mode=responses#some-type-specific-keywords-are-not-yet-supported
                    schema.remove("format");
                    schema.remove("minimum");
                    schema.remove("maximum");
                    schema.remove("minLength");
                    schema.remove("maxLength");
                    schema.remove("multipleOf");
                    schema.remove("patternProperties");
                    schema.remove("unevaluatedProperties");
                    schema.remove("propertyNames");
                    schema.remove("minProperties");
                    schema.remove("maxProperties");
                    schema.remove("unevaluatedItems");
                    schema.remove("minItems");
                    schema.remove("maxItems");
                    schema.remove("minContains");
                    schema.remove("maxContains");
                    schema.remove("contains");
                    schema.remove("uniqueItems");


                    if schema.get("properties").is_some() {
                        schema.insert("additionalProperties".to_owned(), false.into());
                        schema.insert("strict".to_owned(), true.into());
                    }

                    if schema.get("parameters").is_some() {
                        // schema.insert("strict".to_owned(), true.into());
                    }

                    // cant have ref and description
                    if schema.get("$ref").is_some() && schema.get("description").is_some() {
                        schema.remove("description");
                    }

                    if let Some(one_of) = schema.remove("oneOf") {
                        schema.insert("anyOf".to_owned(), one_of);
                    }

                    if let Some(items) = schema.remove("items") {
                        if items.as_bool().is_some() {
                            // any type
                            let new_items = serde_json::json!({
                                "anyOf": [
                                    {"type": "string"},
                                    {"type": "number"},
                                    {"type": "boolean"},
                                    {"type": "array",
                                        "items": {
                                            "type": ["string", "number", "boolean"]
                                        }
                                    }
                                ]});
                            schema.insert("items".to_owned(), new_items);
                        } else {
                            schema.insert("items".to_owned(), items);
                        }
                    }

                    if let Some(prefix) = schema.remove("prefixItems") {
                        schema.remove("maxItems");
                        schema.remove("minItems");
                        let fixed_items = prefix.as_array().unwrap();
                        let mut types = Vec::new();
                        fixed_items.into_iter().for_each(|item| {
                            types.push(
                                item.as_object()
                                    .unwrap()
                                    .get("type")
                                    .unwrap()
                                    .as_str()
                                    .unwrap()
                                    .to_owned(),
                            );
                        });
                        types.sort();
                        types.dedup();

                        schema.insert("items".to_owned(), serde_json::json!({ "type": types}));
                    }
                },
            ))
            .into_generator()
            .into_root_schema_for::<$type>()
    };
}

pub fn llm_json_response_cleanup(llm_response: &str) -> String {
    let mut text_res = llm_response.to_string();

    if text_res.starts_with("<think>") {
        text_res = text_res[8..].to_string();
        // Remove all text until after </think> tag
        if let Some(think_end) = text_res.find("</think>") {
            text_res = text_res[think_end + 8..].to_string();
        }
    }

    if let Some(json_start) = text_res.find("```json") {
        if let Some(json_end) = text_res.rfind("```") {
            // Extract text between ```json and the last ```
            let json_content = &text_res[json_start + 7..json_end].trim();
            text_res = json_content.to_string();
        }
    } else if let Some(json_start) = text_res.find("```") {
        if let Some(json_end) = text_res.rfind("```") {
            // Extract text between ```json and the last ```
            let json_content = &text_res[json_start + 3..json_end].trim();
            text_res = json_content.to_string();
        }
    }
    // Find invalid JSON escape sequences and fix them
    // Valid JSON escape sequences are: \", \\, \/, \b, \f, \n, \r, \t, \u####
    // We need to add a backslash to escape any backslash that isn't part of a valid escape sequence

    // This regex looks for a backslash followed by a character that's not one of the valid escape characters
    let re = regex::Regex::new(r#"\\([^\\/bfnrtu]|u[0-9a-fA-F]{0,3}$|u[0-9a-fA-F]{5,})"#).unwrap();
    text_res = re.replace_all(&text_res, r"\\$1").to_string();
    text_res = text_res.replace(r#"\\""#, r#"\""#);

    text_res
}

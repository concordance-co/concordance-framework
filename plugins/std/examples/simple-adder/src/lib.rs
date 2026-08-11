// Construct the injector plugin interface
wit_bindgen::generate!({
    world: "injector",
    path: "../../../../wit",
    generate_all,
});
use crate::exports::plugin::injector::guest::{
    Guest, GuestJsonToJson, Metadata, PluginError, PluginKind,
};

// host capabilities
use crate::plugin::injector::host::{get, log};
use crate::plugin::injector::http::{HttpRequest, HttpResponse};
use crate::plugin::injector::logger::Level;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use shared::inlined_schema_for;

#[derive(Debug, Default, Deserialize, Serialize, JsonSchema)]
pub struct Adder {
    pub field1: i32,
    pub field2: i32,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct Resp {
    pub result: i32,
}

struct ExampleAdderPlugin;

impl Guest for ExampleAdderPlugin {
    type JsonToJson = ExampleAdder;
    fn get_metadata() -> Metadata {
        Metadata {
            name: "Example Adder Plugin".to_string(),
            version: "0.1.0".to_string(),
            author: "Your Name".to_string(),
            description: "An example of a plugin that adds two numbers".to_string(),
            kind: PluginKind::Tool,
            env_var_support: vec![],
            input_schema: serde_json::to_string(&inlined_schema_for!(Adder)).unwrap(),
            default_input: serde_json::to_string(&Adder::default()).unwrap(),
            output_schema: serde_json::to_string(&inlined_schema_for!(Resp)).unwrap(),
        }
    }
}

struct ExampleAdder;

impl GuestJsonToJson for ExampleAdder {
    fn work(&self, input: String) -> Result<String, PluginError> {
        let Ok(adder) = serde_json::from_str::<Adder>(&input) else {
            return Err(PluginError::Json("Invalid input".to_string()));
        };
        log(Level::Info, &format!("Processing input: {:?}", adder));

        // we can use the host provided http client to make a request
        let res: Result<HttpResponse, PluginError> = get(&HttpRequest {
            url: "http://127.0.0.1:8080/plugins/list".to_string(),
            headers: vec![],
            body: vec![],
        });

        // we can use the host provided logging functionality to log the result of the request
        log(
            Level::Info,
            &format!(
                "Made a http request: {:?}",
                serde_json::from_slice::<serde_json::Value>(&res.unwrap().body).unwrap()
            ),
        );

        // Add the result of the addition to the response
        Ok(serde_json::to_string(&Resp {
            result: adder.field1 + adder.field2,
        })
        .unwrap())
    }

    fn new() -> Self {
        Self {}
    }
}

export!(ExampleAdderPlugin);

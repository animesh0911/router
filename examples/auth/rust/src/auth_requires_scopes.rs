use std::ops::ControlFlow;
use apollo_router::plugin::Plugin;
use apollo_router::plugin::PluginInit;
use apollo_router::register_plugin;
use apollo_router::services::router;
use apollo_router::layers::ServiceBuilderExt;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;
use tower::BoxError;
use tower::ServiceBuilder;
use tower::ServiceExt;

#[derive(Debug)]
struct Auth {
    #[allow(dead_code)]
    configuration: Conf,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
struct Conf {
}

// This is a bare bones plugin that can be duplicated when creating your own.
#[async_trait::async_trait]
impl Plugin for Auth {
    type Config = Conf;

    async fn new(init: PluginInit<Self::Config>) -> Result<Self, BoxError> {
        Ok(Auth {
            configuration: init.config,
        })
    }

    fn router_service(&self, service: router::BoxService) -> router::BoxService {
        // Always use service builder to compose your plugins.
        // It provides off the shelf building blocks for your plugin.
        ServiceBuilder::new()
            .checkpoint_async(move |request: router::Request| {
                async move {
                    return Ok(ControlFlow::Continue(enrich_request_context_with_auth_information(request)))
                }
            })
            .buffered()
            .service(service)
            .boxed()
    }
}

fn enrich_request_context_with_auth_information(request: router::Request) -> router::Request {
    let claims: serde_json::Value = json!({
        "scopes": "booking_default address",
        "acr": "AAL1",
    });
    println!("CLAIMS: {}", claims);
    request.context
        .insert("apollo_authentication::JWT::claims", claims)
        .expect("failed to set claims in request context");
    request
}

// This macro allows us to use it in our plugin registry!
// register_plugin takes a group name, and a plugin name.
//
// In order to keep the plugin names consistent,
// we use using the `Reverse domain name notation`
register_plugin!("example", "auth_requires_scopes", Auth);
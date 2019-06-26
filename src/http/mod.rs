use crate::storage::DefaultStorage;
use config::Config;
use rouille::{router, start_server, Response};

mod api;

pub fn run(config: &Config) -> ! {
    let addr = config
        .get_str("http.addr")
        .unwrap_or_else(|_| "0.0.0.0:3000".into());
    let context = Context::load(config);
    eprintln!("listening on address {}...", addr);
    start_server(addr, move |request| {
        router!(request,
            (POST)["/api/recommend"] => {
                api::recommend::apply(request, &context).into()
            },
            _ => { Response::empty_404() }
        )
    })
}

pub struct Context<'c> {
    config: &'c Config,
    storage: DefaultStorage,
}

impl<'c> Context<'c> {
    pub fn load(config: &'c Config) -> Context<'c> {
        Context {
            config,
            storage: DefaultStorage::load(config),
        }
    }
}
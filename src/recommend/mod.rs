pub use self::conf::PartConfig;
pub use self::request::Request;
use crate::learn::logistic::Parameters;
use crate::storage::{Activity, BasicExample, Example, FeatureList, Store};
use config::Config;
use failure::Error;
use rand::Rng;
use std::borrow::Borrow;
use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

mod conf;
mod request;
mod train;

#[derive(Debug, Clone)]
pub struct Core<T: Store + 'static> {
    pub storage: Arc<T>,
    pub parameters: Parameters<f64>,
    pub part_config: HashMap<String, PartConfig>,
    pub default_config: PartConfig,
}

impl<T: Store + 'static> Core<T> {
    pub fn of(storage: &Arc<T>, config: &Config) -> Core<T> {
        let default_config = config.get("recommend.core.default").unwrap_or_default();
        let part_config = config.get("recommend.core.parts").unwrap_or_default();
        let parameters = config.get("recommend.core.parameters").unwrap_or_default();
        Core {
            storage: storage.clone(),
            parameters,
            part_config,
            default_config,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub items: Vec<(Uuid, f64)>,
    pub id: Uuid,
}

impl<T: Store + 'static> Core<T> {
    pub fn config_for<Q>(&self, name: &Q) -> &PartConfig
    where
        String: Borrow<Q>,
        Q: Hash + Eq,
    {
        self.part_config
            .get(name)
            .unwrap_or_else(|| &self.default_config)
    }

    pub fn recommend(&self, request: &Request) -> Result<Response, Error> {
        let current_item = request.current(self)?;
        debug!("current_item={:?}", current_item);
        let current = Example::new(BasicExample::new(current_item.id), current_item);
        let config = self.config_for(&request.part);
        debug!("config={:?}", config);
        let model = pluck_model(self.storage.as_ref(), &request.part)?;
        debug!("model={:?}", model);
        let examples = request.examples(self)?;
        debug!("examples=impl");
        let mut scored = score_examples(examples, &current, &model, config).collect::<Vec<_>>();
        debug!("scored={:?}", scored);
        crate::ord::sort_float(&mut scored, |(_, a)| *a);
        resort_examples(&mut scored, request.count, config);
        scored.truncate(request.count);
        debug!("scored.truncate");

        let id = build_activity(self.storage.as_ref(), request, current, &scored[..])?;
        debug!("id={:?}", id);

        Ok(Response {
            items: scored.into_iter().map(|(v, s)| (v.item.id, s)).collect(),
            id,
        })
    }
}

impl<T: Store + Send + Sync + 'static> Core<T> {
    pub fn train_loop(core: &Arc<Self>) -> std::thread::JoinHandle<()> {
        let core = core.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_secs(60 * 10));
            info!("performing load_train...");
            match core.load_train() {
                Ok(_) => info!("load_train successful!"),
                Err(e) => {
                    error!("error occurred during train_loop: {:?}", e);
                }
            }
        })
    }
}

fn pluck_model<T: Store>(storage: &T, part: &str) -> Result<FeatureList<'static>, Error> {
    let model = storage.find_model(part)?;
    match model {
        Some(model) => Ok(model),
        None => storage.find_default_model(),
    }
}

fn score_examples<'v, I>(
    examples: I,
    current: &'v Example,
    model: &'v FeatureList<'static>,
    config: &'v PartConfig,
) -> impl Iterator<Item = (Example, f64)> + 'v
where
    I: Iterator<Item = Example> + 'v,
{
    use crate::learn::logistic::predict_iter;
    examples.map(move |example| {
        let features = example.features(&current, config);
        let iter = features.combine(&model).map(|(_, a, b)| (a, b));
        let score = predict_iter::<f64, _>(iter);
        (example, score)
    })
}

fn build_activity<T: Store>(
    storage: &T,
    request: &Request,
    current: Example,
    visible: &[(Example, f64)],
) -> Result<Uuid, Error> {
    let activity_id = Uuid::new_v4();

    let visible = visible.iter().map(|(e, _)| e.clone()).collect::<Vec<_>>();

    let activity = Activity {
        id: activity_id,
        part: request.part.clone(),
        current,
        visible,
        chosen: None,
    };

    storage.model_activity_save(&request.part, &activity)?;
    Ok(activity_id)
}

fn resort_examples(examples: &mut Vec<(Example, f64)>, max: usize, config: &PartConfig) {
    if max >= examples.len() {
        return;
    }

    let mut rng = rand::thread_rng();

    if !rng.gen_bool(config.upgrade_chance) {
        return;
    }

    let from = rng.gen_range(max, examples.len());
    let to = rng.gen_range(0, max);

    examples.swap(to, from);
}

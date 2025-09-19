use std::time::Instant;
use indexmap::IndexMap;

#[derive(Debug)]
pub struct RequestProfiling {
    start: Instant,
    marks: IndexMap<String, u64>, // action -> millis since start
}

impl RequestProfiling {
    pub fn new() -> Self {
        let start = Instant::now();
        Self {
            start,
            marks: IndexMap::new(),
        }
    }

    pub fn mark(&mut self, name: &str) {
        let elapsed: u64 = self.start.elapsed().as_nanos().try_into().unwrap();
        self.marks.insert(name.to_string(), elapsed);
    }

    pub fn get_report(&self) -> serde_json::Value {
        let mut report = serde_json::Map::new();
        for (action, ts) in &self.marks {
            report.insert(action.clone(), serde_json::Value::Number(serde_json::Number::from(*ts)));
        }
        serde_json::Value::Object(report)
    }


    pub fn log_report(&self) {


        let mut report = String::new();
        report.push_str("--- Request timing report ---\n");

        let mut previous_time = 0;

        for (action, ts) in &self.marks {
            let duration = (ts - previous_time) as f64 / 1000000.0;

            report.push_str(&format!("{:<10} {} ms\n", action, duration));

            previous_time = *ts;
        }

        report.push_str("-----------------------------");

        log::info!("{}", report);
    }

}
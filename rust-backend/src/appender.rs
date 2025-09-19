//! The file appender.
//!
//! Requires the `file_appender` feature.

use derivative::Derivative;
use log::Record;
use std::{
    io::{self}, borrow::BorrowMut
};

#[cfg(feature = "config_parsing")]
use log4rs::config::{Deserialize, Deserializers};
#[cfg(feature = "config_parsing")]
use log4rs::encode::EncoderConfig;

use log4rs::{
    append::Append,
    encode::{pattern::PatternEncoder, Encode},
};

struct Writable {
    buffer: Vec<u8>
}

impl io::Write for Writable {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.buffer.write(buf).unwrap();
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.buffer.flush()
    }
}

impl log4rs::encode::Write for Writable {
    fn set_style(&mut self, _: &log4rs::encode::Style) -> io::Result<()> {
        Ok(())
    }
}

impl Writable {
    pub fn create() -> Writable {
        Writable{
            buffer: Vec::new()
        }
    }
    // to string
    fn to_string(&self) -> String {
        String::from(std::str::from_utf8(&self.buffer).unwrap())
    }
}

/// The file appender's configuration.
#[cfg(feature = "config_parsing")]
#[derive(Clone, Eq, PartialEq, Hash, Debug, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebAppenderConfig {
    api_endpoint: String,
    encoder: Option<EncoderConfig>
}

/// An appender which logs to a file.
#[derive(Derivative)] 
#[derivative(Debug)]
pub struct WebAppender {
    api_endpoint: String,
    #[derivative(Debug = "ignore")]
    encoder: Box<dyn Encode>,
    #[derivative(Debug = "ignore")]
    client: reqwest::blocking::Client
}

impl Append for WebAppender {
    fn append(&self, record: &Record) -> anyhow::Result<()> {
        let mut writer = Writable::create();
        self.encoder.encode(writer.borrow_mut(), record)?;
        self.send_log(writer.to_string())?;
        Ok(())
    }

    fn flush(&self) {}
}

impl WebAppender {
    /// Creates a new `WebAppender` builder.
    #[allow(dead_code)]
    pub fn builder() -> WebAppenderBuilder {
        WebAppenderBuilder {
            encoder: None,
        }
    }

    fn send_log(&self, log: String) -> anyhow::Result<()> {

        let form_data = [("log", log)];
        
        self.client
            .post(&self.api_endpoint)
            .form(&form_data).send()?;

        Ok(())
    }
}

#[allow(dead_code)]
/// A builder for `WebAppender`s.
pub struct WebAppenderBuilder {
    encoder: Option<Box<dyn Encode>>
}

impl WebAppenderBuilder {
    /// Sets the output encoder for the `WebAppender`.
    #[allow(dead_code)]
    pub fn encoder(mut self, encoder: Box<dyn Encode>) -> WebAppenderBuilder {
        self.encoder = Some(encoder);
        self
    }

    /// Consumes the `WebAppenderBuilder`, producing a `WebAppender`.
    #[allow(dead_code)]
    pub fn build(self, api_endpoint: &str) -> io::Result<WebAppender> {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build().unwrap();

        Ok(WebAppender {
            api_endpoint: String::from(api_endpoint),
            encoder: self
                .encoder
                .unwrap_or_else(|| Box::new(PatternEncoder::default())),
            client
        })
    }
}

/// A deserializer for the `WebAppender`.
///
/// # Configuration
///
/// ```yaml
/// kind: file
///
/// # The api_endpoint of the log API. Required.
/// api_endpoint: log/foo.log
///
/// # The encoder to use to format output. Defaults to `kind: pattern`.
/// encoder:
///   kind: pattern
/// ```
#[cfg(feature = "config_parsing")]
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Default)]
pub struct WebAppenderDeserializer;

#[cfg(feature = "config_parsing")]
impl Deserialize for WebAppenderDeserializer {
    type Trait = dyn Append;

    type Config = WebAppenderConfig;

    fn deserialize(
        &self,
        config: WebAppenderConfig,
        deserializers: &Deserializers,
    ) -> anyhow::Result<Box<Self::Trait>> {
        let mut appender = WebAppender::builder();
        
        if let Some(encoder) = config.encoder {
            appender = appender.encoder(deserializers.deserialize(&encoder.kind, encoder.config)?);
        }
        
        Ok(Box::new(appender.build(&config.api_endpoint)?))
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn create_directories() {
        let api_endoint = String::from("http://localhost:8080/api/v1/logs");

        WebAppender::builder()
            .build(&api_endoint)
            .unwrap();
    }

}
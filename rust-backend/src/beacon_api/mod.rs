use futures::StreamExt;
use reqwest::StatusCode;
use std::string::ToString;
use std::{
    fs::{self},
    io::{Write},
}; 




pub async fn query(
    query: &str,
    beacon_url: &str,
    auth_token: &str,
    file_path: &str,
) -> Result<(String, String), (String, i32)> {
    //create client with long timeout:
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(600))
        .build()
        .map_err(|e| (e.to_string(), 500))?;

    log::info!(
        "Querying beacon with query: {}/api/query {}", beacon_url,
        query
    );

    let query_url = format!("{beacon_url}/api/query");

    let mut rb = client
        .post(query_url)
        .body(query.to_string())
        .header("Content-Type", "application/json");

    if auth_token != "" {
        rb = rb.header("Authorization", format!("Bearer {}", auth_token));
    }

    let response = rb.send().await.map_err(|e| (e.to_string(), 500))?;

    //store response directly to file if it's 200:
    let headers = response.headers().clone();
    let query_id = match headers.get("x-beacon-query-id") {
        Some(header) => header.to_str().map_err(|e| (e.to_string(), 500))?,
        None => "",
    };

    log::info!("Query headers: {:?}", headers);

    match response.status() {
        StatusCode::OK => {
            let mut file = fs::File::create(file_path)
                .map_err(|e| (e.to_string(), 500))?;
            
            let mut stream = response.bytes_stream();

            while let Some(chunk) = stream.next().await {
                // let chunk = chunk.map_err(|e| (e.to_string(), 500))?;
                // file.write_all(&chunk).map_err(|e| (e.to_string(), 500))?;
                let chunk = match chunk {
                    Ok(bytes) => bytes,
                    Err(e) => {
                        // cleanup partial file
                        let _ = fs::remove_file(file_path);
                        return Err((e.to_string(), 500));
                    }
                };
            
                if let Err(e) = file.write_all(&chunk) {
                    let _ = fs::remove_file(file_path);
                    return Err((e.to_string(), 500));
                }
            }

            // if file is empty/no chunks were read then delete the file and return err
            if fs::metadata(file_path).unwrap().len() == 0 {
                fs::remove_file(file_path)
                    .map_err(|e| (e.to_string(), 500))?;
                return Err(("Returned query has file lenght 0".to_string(), 500));
            }

        }

        StatusCode::NO_CONTENT => {
            log::info!("No content found for query");
            return Err(("No content found for query".to_string(), 204));
        }

        _ => {
            let status = response.status();
            let content = response.text().await.map_err(|e| (e.to_string(), 500))?;
            log::error!(
                "Beacon query failed with status code: {}: \n{}",
                status, content
            );
            return Err((
                format!(
                    "Beacon query failed with status code: {}: \n{}",
                    status, content
                ),
                status.as_u16() as i32,
            ));
        }
    }

    Ok((file_path.to_string(), query_id.to_string()))
}



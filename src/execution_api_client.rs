use reqwest::Client;

pub struct ExecutionApiClient {
    pub client: Client,
    pub base_url: String,
    pub jwt_secret: String,
}
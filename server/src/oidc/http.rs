//! `openidconnect` 용 HTTP 클라이언트. 기존 reqwest 0.13 사용(라이브러리 내장 reqwest 버전 중복 회피).

use std::{future::Future, pin::Pin, sync::LazyLock, time::Duration};

use openidconnect::{AsyncHttpClient, HttpClientError, HttpRequest, HttpResponse, http};

/// 요청 1회 한도. 설정 저장과 로그인이 IdP 장애에 묶이지 않게.
pub const TIMEOUT: Duration = Duration::from_secs(10);
/// 응답 본문 상한. discovery, JWKS, token 응답은 수 KiB. 비로그인 callback 에서도 조회.
pub const MAX_BODY: usize = 1024 * 1024;

// issuer 는 owner 입력. 리다이렉트로 다른 호스트에 닿지 않게.
static SHARED: LazyLock<reqwest::Result<Client>> = LazyLock::new(|| {
    reqwest::Client::builder()
        .timeout(TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map(Client)
});

pub struct Client(reqwest::Client);

impl Client {
    /// 프로세스 공용(연결 재사용).
    pub fn shared() -> Result<&'static Client, &'static reqwest::Error> {
        SHARED.as_ref()
    }
}

impl<'c> AsyncHttpClient<'c> for Client {
    type Error = HttpClientError<reqwest::Error>;
    type Future = Pin<Box<dyn Future<Output = Result<HttpResponse, Self::Error>> + Send + 'c>>;

    fn call(&'c self, request: HttpRequest) -> Self::Future {
        Box::pin(async move {
            let request = reqwest::Request::try_from(request).map_err(Box::new)?;
            let mut response = self.0.execute(request).await.map_err(Box::new)?;
            if response
                .content_length()
                .is_some_and(|n| n > MAX_BODY as u64)
            {
                return Err(too_large());
            }
            let mut builder = http::Response::builder()
                .status(response.status())
                .version(response.version());
            for (name, value) in response.headers() {
                builder = builder.header(name, value);
            }
            let mut body = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(Box::new)? {
                if body.len() + chunk.len() > MAX_BODY {
                    return Err(too_large());
                }
                body.extend_from_slice(&chunk);
            }
            builder.body(body).map_err(HttpClientError::Http)
        })
    }
}

fn too_large() -> HttpClientError<reqwest::Error> {
    HttpClientError::Other(format!("response body too large (over {MAX_BODY} bytes)"))
}

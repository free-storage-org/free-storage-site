use js_sys::Promise;
use serde::{Deserialize, Serialize};
use wasm_bindgen::{prelude::*, JsCast};
use wasm_bindgen_futures::JsFuture;
use web_sys::{Document, Event, FormData, HtmlFormElement, RequestInit, Response, Window};
use yew::{html, use_effect};

#[global_allocator]
static ALLOC: wee_alloc::WeeAlloc = wee_alloc::WeeAlloc::INIT;

#[derive(Clone, Debug, Deserialize, Serialize)]
struct FileId {
    pub asset_url: String,
    pub chunks: usize,
}

#[yew::function_component(Hello)]
fn hello() -> Html {
    use_effect(|| {
        let form = document()
            .query_selector("#upload-form")
            .unwrap()
            .unwrap()
            .unchecked_into::<HtmlFormElement>();

        form.add_event_listener_with_callback(
            "submit",
            Closure::<dyn Fn(Event)>::new(|e: Event| {
                e.prevent_default();
                let form = e.target().unwrap().unchecked_into::<HtmlFormElement>();
                let output = document().query_selector("#output").unwrap().unwrap();
                wasm_bindgen_futures::spawn_local(async move {
                    let mut req_opts = RequestInit::new();
                    req_opts.method("POST");
                    req_opts.body(Some(FormData::new_with_form(&form).unwrap().as_ref()));

                    let res = f(f(window().fetch_with_str_and_init("/upload", &req_opts))
                        .await
                        .unwrap()
                        .unchecked_into::<Response>()
                        .json()
                        .unwrap())
                    .await
                    .unwrap()
                    .into_serde::<Vec<FileId>>()
                    .unwrap();

                    for file_id in res {
                        let url = format!(
                            "{origin}/get?{querystring}",
                            origin = window().location().origin().unwrap(),
                            querystring = serde_qs::to_string(&file_id).unwrap()
                        );
                        let a = document().create_element("a").unwrap();
                        a.set_attribute("href", &url).unwrap();
                        a.set_inner_html(&url);
                        output.append_child(&a).unwrap();

                        output
                            .append_child(&document().create_element("br").unwrap())
                            .unwrap();
                    }
                })
            })
            .into_js_value()
            .as_ref()
            .unchecked_ref(),
        )
        .unwrap();

        || {}
    });

    html! {
        <div>
            <form id="upload-form">
                <input type="file" name="file" id="files" multiple={true} />
                <input type="submit" value="Upload" />
            </form>
            <div>
                {"Your download URLs are:"}
                <span id="output"></span>
            </div>
        </div>
    }
}

#[wasm_bindgen(start)]
pub fn main() {
    yew::start_app_in_element::<Hello>(
        web_sys::window()
            .unwrap()
            .document()
            .unwrap()
            .get_element_by_id("app")
            .unwrap(),
    );
}

fn window() -> Window {
    web_sys::window().unwrap()
}

fn document() -> Document {
    window().document().unwrap()
}

fn f(p: Promise) -> JsFuture {
    JsFuture::from(p)
}

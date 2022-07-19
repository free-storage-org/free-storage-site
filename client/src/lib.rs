use wasm_bindgen::prelude::*;
use yew::html;

#[global_allocator]
static ALLOC: wee_alloc::WeeAlloc = wee_alloc::WeeAlloc::INIT;

#[yew::function_component(Hello)]
fn hello() -> Html {
    html! {
        <div>
            <form enctype="multipart/form-data" action="/upload" method="POST">
                <input type="file" name="file" />
                <input type="submit" value="Upload" />
            </form>
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

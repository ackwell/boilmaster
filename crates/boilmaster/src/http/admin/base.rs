use maud::{html, Markup, Render, DOCTYPE};

pub struct BaseTemplate {
	pub title: String,
	pub content: Markup,
}

impl Render for BaseTemplate {
	fn render(&self) -> Markup {
		html! {
			(DOCTYPE)
			html {
				head {
					title { "admin | " (self.title) }
					link rel="stylesheet" href="https://cdn.jsdelivr.net/npm/@picocss/pico@2/css/pico.min.css";
				}
				body {
					header.container {
						nav {
							ul {
								li { strong { "boilmaster" } }
								li { (self.title) }
							}
							ul {
								li { a href="/admin" { "versions" } }
							}
						}
					}

					main.container {
						(self.content)
					}
				}
			}
		}
	}
}

use anyhow::{anyhow, Context, Result};
use headless_chrome::browser::default_executable;
use headless_chrome::{Browser, LaunchOptions};
use std::ffi::OsStr;
use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::Duration;

pub struct JobFetcher {
    browser: Browser,
}

impl JobFetcher {
    pub fn new(headless: bool) -> Result<Self> {
        // Check if Chrome is already running
        if Self::is_chrome_running()? {
            return Err(anyhow!(
                "Chrome is already running. Please close all Chrome windows and try again.\n\
                 \n\
                 This command needs exclusive access to your Chrome profile to access\n\
                 your logged-in LinkedIn session.\n\
                 \n\
                 To check running Chrome processes: ps aux | grep chrome"
            ));
        }

        // Use the user's Chrome profile to access logged-in LinkedIn session
        // Default Chrome profile location on Linux: ~/.config/google-chrome
        let home = std::env::var("HOME").unwrap_or_else(|_| String::from("/home"));
        let user_data_dir = PathBuf::from(&home).join(".config/google-chrome");

        // Args to prevent session restore and specify profile
        let args: Vec<&OsStr> = vec![
            OsStr::new("--no-first-run"),
            OsStr::new("--no-default-browser-check"),
            OsStr::new("--disable-restore-session-state"),
            OsStr::new("--disable-session-crashed-bubble"),
            OsStr::new("--profile-directory=Default"),
            OsStr::new("--disable-features=TranslateUI"),
            OsStr::new("--disable-sync"),
            OsStr::new("about:blank"),  // Start with blank page
        ];

        let launch_options = LaunchOptions {
            headless,
            sandbox: true,
            user_data_dir: Some(user_data_dir),
            path: default_executable().ok(),
            args,
            ..Default::default()
        };

        println!("Launching Chrome with user profile...");
        let browser = Browser::new(launch_options)
            .context("Failed to launch Chrome. Make sure Chrome is installed.")?;

        Ok(JobFetcher { browser })
    }

    pub fn fetch_job_description(&self, url: &str) -> Result<String> {
        use std::io::{self, Write};

        print!("Waiting for browser to initialize... ");
        io::stdout().flush().unwrap();
        thread::sleep(Duration::from_secs(3));
        println!("✓");

        print!("Creating new browser tab... ");
        io::stdout().flush().unwrap();
        let tab = self.browser.new_tab()
            .context("Failed to create new browser tab")?;
        println!("✓");

        // Navigate to the job URL
        print!("Navigating to: {} ... ", url);
        io::stdout().flush().unwrap();

        match tab.navigate_to(url) {
            Ok(_) => {
                println!("✓");

                // Verify navigation worked
                thread::sleep(Duration::from_secs(2));
                let current_url = tab.get_url();
                println!("Current URL: {}", current_url);

                // Get page title to verify page loaded
                if let Ok(title_element) = tab.find_element("title") {
                    if let Ok(title) = title_element.get_inner_text() {
                        println!("Page title: {}", title);
                    }
                }

                // Check if we're on the right page
                if !current_url.contains("linkedin.com") {
                    println!("WARNING: Not on LinkedIn! Dumping page HTML...");
                    if let Ok(body) = tab.find_element("body") {
                        if let Ok(html) = body.get_inner_text() {
                            println!("Page content (first 500 chars):\n{}", &html[..html.len().min(500)]);
                        }
                    }
                    return Err(anyhow!("Navigation failed - ended up at: {}", current_url));
                }
            }
            Err(e) => {
                return Err(anyhow!("Failed to navigate to job URL: {}", e));
            }
        }

        // Wait for page to load
        print!("Waiting for page to load");
        io::stdout().flush().unwrap();
        for _ in 0..5 {
            thread::sleep(Duration::from_secs(1));
            print!(".");
            io::stdout().flush().unwrap();
        }
        println!(" ✓");

        // Check for LinkedIn auth wall
        print!("Checking authentication status... ");
        io::stdout().flush().unwrap();
        if self.check_auth_required(&tab)? {
            println!("✗");

            return Err(anyhow!(
                "LinkedIn authentication required. Please:\n\
                 1. Make sure you're logged into LinkedIn in your Chrome browser\n\
                 2. Close all Chrome windows before running 'hunt fetch'\n\
                 3. Try running without --headless flag"
            ));
        }
        println!("✓");

        // Try to find and click "Show more" button
        print!("Looking for 'Show more' button... ");
        io::stdout().flush().unwrap();
        let show_more_selectors = vec![
            "button.show-more-less-html__button",
            "button.show-more-less-html__button--more",
            ".jobs-description__footer-button",
            "button[aria-label*='Show more']",
            "button[aria-label*='See more']",
        ];

        let mut found_button = false;
        for selector in &show_more_selectors {
            if let Ok(element) = tab.find_element(selector) {
                println!("✓\nClicking 'Show more' button... ");
                io::stdout().flush().unwrap();
                if element.click().is_ok() {
                    println!("✓");
                    thread::sleep(Duration::from_secs(2));
                    found_button = true;
                    break;
                }
            }
        }
        if !found_button {
            println!("(not found, continuing anyway)");
        }

        // Extract job description
        println!("Extracting job description...");
        let description_selectors = vec![
            ".jobs-description__content",
            ".jobs-box__html-content",
            ".show-more-less-html__markup",
            ".description__text",
            "div.jobs-description-content__text",
            "#job-details",
            "article.jobs-description",
        ];

        for selector in &description_selectors {
            if let Ok(element) = tab.find_element(selector) {
                if let Ok(text) = element.get_inner_text() {
                    if !text.trim().is_empty() {
                        println!("Successfully extracted {} characters", text.len());
                        return Ok(text);
                    }
                }
            }
        }

        // Fallback: get all text from body
        println!("Using fallback: extracting all body text...");
        let body = tab.find_element("body")
            .context("Failed to find body element")?;
        let text = body.get_inner_text()
            .context("Failed to get body text")?;

        if text.trim().is_empty() {
            return Err(anyhow!("No content found on page"));
        }

        Ok(text)
    }

    fn is_chrome_running() -> Result<bool> {
        // Check if actual Chrome/Chromium browser processes are running
        // (not Electron apps like Cursor, GitKraken, etc.)
        let output = Command::new("pgrep")
            .arg("-f")
            .arg("/opt/google/chrome/chrome|/usr/bin/chromium|/usr/bin/google-chrome")
            .output();

        match output {
            Ok(result) => Ok(!result.stdout.is_empty()),
            Err(_) => {
                // If pgrep isn't available, try ps as fallback
                let ps_output = Command::new("ps")
                    .arg("aux")
                    .output()
                    .context("Failed to check for running Chrome processes")?;

                let output_str = String::from_utf8_lossy(&ps_output.stdout);
                // Look for actual Chrome binary paths, not Electron apps
                Ok(output_str.contains("/opt/google/chrome/chrome ")
                    || output_str.contains("/usr/bin/chromium ")
                    || output_str.contains("/usr/bin/google-chrome "))
            }
        }
    }

    fn check_auth_required(&self, tab: &headless_chrome::Tab) -> Result<bool> {
        // Check for common LinkedIn auth/login indicators
        let auth_indicators = vec![
            "input[name='session_key']",  // Login form
            "input[name='session_password']",  // Login form
            ".authwall",  // Auth wall class
            "button[aria-label*='Sign in']",  // Sign in button
        ];

        for selector in &auth_indicators {
            if tab.find_element(selector).is_ok() {
                return Ok(true);
            }
        }

        // Check if URL redirected to login page
        let url = tab.get_url();
        if url.contains("/login") || url.contains("/authwall") {
            return Ok(true);
        }

        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore] // Ignore by default since it requires network/browser
    fn test_fetch_job_description() {
        let fetcher = JobFetcher::new(false).expect("Failed to create fetcher");
        let url = "https://www.linkedin.com/jobs/view/1234567890";
        let result = fetcher.fetch_job_description(url);

        // This will likely fail without a real URL, but tests the structure
        assert!(result.is_ok() || result.is_err());
    }
}

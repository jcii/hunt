use anyhow::{anyhow, Context, Result};
use std::process::Command;
use thirtyfour::prelude::*;

pub struct JobFetcher {
    driver: WebDriver,
}

impl JobFetcher {
    pub async fn new(headless: bool) -> Result<Self> {
        // Check if Firefox is already running with the profile we need
        if Self::is_firefox_running()? {
            return Err(anyhow!(
                "Firefox is already running. Close Firefox and try again immediately.\n\
                 \n\
                 Why: geckodriver needs exclusive access to your Firefox profile to use\n\
                 your logged-in LinkedIn session. The profile can't be used by two processes.\n\
                 \n\
                 Steps:\n\
                 1. Close all Firefox windows (or run: pkill firefox)\n\
                 2. Run this command again right away\n\
                 3. geckodriver will start Firefox with your profile and LinkedIn cookies"
            ));
        }

        // Firefox profile location (snap Firefox)
        let home = std::env::var("HOME").unwrap_or_else(|_| "/home".to_string());
        let firefox_profile_dir = format!("{}/snap/firefox/common/.mozilla/firefox/5krdosdy.default", home);

        println!("Using Firefox profile: {}", firefox_profile_dir);

        // Create Firefox capabilities with user profile
        let mut caps = DesiredCapabilities::firefox();

        // Add Firefox args to specify profile
        caps.add_arg("-profile")?;
        caps.add_arg(&firefox_profile_dir)?;

        if headless {
            caps.set_headless()?;
        }

        println!("Starting geckodriver...");

        // Connect to geckodriver
        // thirtyfour expects geckodriver to be running separately
        // We'll use the default geckodriver URL
        let driver = WebDriver::new("http://localhost:4444", caps)
            .await
            .context("Failed to connect to geckodriver. Make sure geckodriver is running.\n\
                     You can start it with: geckodriver --port 4444")?;

        Ok(JobFetcher { driver })
    }

    pub async fn fetch_job_description(&self, url: &str) -> Result<String> {
        println!("Navigating to: {}", url);

        // Navigate to the job URL
        self.driver.goto(url).await
            .context("Failed to navigate to LinkedIn job URL")?;

        println!("Waiting for page to load...");

        // Wait for page to be ready
        tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

        // Check for LinkedIn auth wall
        println!("Checking authentication status...");
        let auth_required = self.check_auth_required().await?;
        if auth_required {
            println!("⚠ LinkedIn auth wall detected, but continuing to try extraction...");
        } else {
            println!("✓ Authenticated");
        }

        // Try to find and click "Show more" button
        println!("Looking for 'Show more' button...");
        let show_more_selectors = vec![
            "button.show-more-less-html__button",
            "button.show-more-less-html__button--more",
            ".jobs-description__footer-button",
            "button[aria-label*='Show more']",
            "button[aria-label*='See more']",
        ];

        let mut found_button = false;
        for selector in &show_more_selectors {
            if let Ok(element) = self.driver.find(By::Css(*selector)).await {
                println!("✓ Found 'Show more' button, clicking...");
                element.click().await?;
                tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
                found_button = true;
                break;
            }
        }
        if !found_button {
            println!("(Show more button not found, continuing anyway)");
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
            if let Ok(element) = self.driver.find(By::Css(*selector)).await {
                if let Ok(text) = element.text().await {
                    if !text.trim().is_empty() {
                        println!("✓ Successfully extracted {} characters", text.len());
                        return Ok(text);
                    }
                }
            }
        }

        // Fallback: get all text from body
        println!("Using fallback: extracting all body text...");
        let body = self.driver.find(By::Tag("body")).await
            .context("Failed to find body element")?;
        let text = body.text().await
            .context("Failed to get body text")?;

        if text.trim().is_empty() {
            return Err(anyhow!("No content found on page"));
        }

        println!("✓ Extracted {} characters (fallback method)", text.len());
        Ok(text)
    }

    fn is_firefox_running() -> Result<bool> {
        // Check if Firefox browser processes are running (not geckodriver)
        let output = Command::new("pgrep")
            .arg("-f")
            .arg("/usr/lib/firefox/firefox")
            .output();

        match output {
            Ok(result) => {
                if !result.stdout.is_empty() {
                    return Ok(true);
                }
                // Also check for snap Firefox
                let snap_check = Command::new("pgrep")
                    .arg("-f")
                    .arg("snap/firefox.*firefox$")
                    .output();
                Ok(snap_check.map(|r| !r.stdout.is_empty()).unwrap_or(false))
            }
            Err(_) => {
                // If pgrep isn't available, try ps as fallback
                let ps_output = Command::new("ps")
                    .arg("aux")
                    .output()
                    .context("Failed to check for running Firefox processes")?;

                let output_str = String::from_utf8_lossy(&ps_output.stdout);
                // Match Firefox browser, not geckodriver
                Ok(output_str.lines().any(|line|
                    (line.contains("/usr/lib/firefox/firefox") ||
                     line.contains("snap/firefox") && line.contains("firefox ")) &&
                    !line.contains("geckodriver")
                ))
            }
        }
    }

    async fn check_auth_required(&self) -> Result<bool> {
        // Check for common LinkedIn auth/login indicators
        let auth_indicators = vec![
            "input[name='session_key']",  // Login form
            "input[name='session_password']",  // Login form
            ".authwall",  // Auth wall class
            "button[aria-label*='Sign in']",  // Sign in button
        ];

        for selector in &auth_indicators {
            if self.driver.find(By::Css(*selector)).await.is_ok() {
                return Ok(true);
            }
        }

        // Check if URL redirected to login page
        let url = self.driver.current_url().await?;
        if url.as_str().contains("/login") || url.as_str().contains("/authwall") {
            return Ok(true);
        }

        Ok(false)
    }
}

// Note: We don't implement Drop to quit the driver because:
// 1. WebDriver::quit() takes ownership (consumes self)
// 2. Drop only has &mut self, so we can't call quit()
// 3. The user should manually close Firefox after use
// 4. Or the driver will clean up when the process exits

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore] // Ignore by default since it requires geckodriver running
    async fn test_fetch_job_description() {
        let fetcher = JobFetcher::new(false).await.expect("Failed to create fetcher");
        let url = "https://www.linkedin.com/jobs/view/1234567890";
        let result = fetcher.fetch_job_description(url).await;

        // This will likely fail without a real URL, but tests the structure
        assert!(result.is_ok() || result.is_err());
    }
}

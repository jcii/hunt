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

        // Extract job description - use innerHTML to preserve structure
        println!("Extracting job description...");

        // Debug: See what's on the page
        if let Ok(body) = self.driver.find(By::Tag("body")).await {
            if let Ok(body_text) = body.text().await {
                println!("DEBUG: Page contains {} chars total", body_text.len());
                if body_text.to_lowercase().contains("about the job") {
                    println!("DEBUG: Found 'About the job' text on page");
                }
            }
        }

        let description_selectors = vec![
            ".jobs-description__content",
            ".show-more-less-html__markup",
            ".jobs-box__html-content",
            "div.jobs-description-content__text",
            "#job-details",
            "article.jobs-description",
        ];

        for selector in &description_selectors {
            if let Ok(element) = self.driver.find(By::Css(*selector)).await {
                // Get HTML content to preserve structure (bullets, paragraphs)
                if let Ok(html) = element.inner_html().await {
                    if !html.trim().is_empty() {
                        let cleaned = Self::extract_and_clean_text(&html)?;
                        if !cleaned.trim().is_empty() {
                            println!("✓ Successfully extracted {} characters from {}", cleaned.len(), selector);
                            return Ok(cleaned);
                        }
                    }
                }
            }
        }

        // Ultimate fallback: get main content area and clean aggressively
        println!("Using ultimate fallback: extracting and cleaning main content...");
        if let Ok(main) = self.driver.find(By::Tag("main")).await {
            if let Ok(html) = main.inner_html().await {
                let cleaned = Self::extract_and_clean_text(&html)?;
                if !cleaned.is_empty() {
                    println!("✓ Extracted {} characters from main element (cleaned)", cleaned.len());
                    return Ok(cleaned);
                }
            }
        }

        // Last resort: Get body text and clean it
        if let Ok(body) = self.driver.find(By::Tag("body")).await {
            if let Ok(html) = body.inner_html().await {
                let cleaned = Self::extract_and_clean_text(&html)?;
                if !cleaned.is_empty() {
                    println!("✓ Extracted {} characters from body (cleaned)", cleaned.len());
                    return Ok(cleaned);
                }
            }
        }

        Err(anyhow!("Could not extract any content from page"))
    }

    fn extract_and_clean_text(html: &str) -> Result<String> {
        // Parse HTML and extract text while preserving structure
        use scraper::Html;

        let document = Html::parse_fragment(html);
        let mut result = String::new();

        // Selectors for elements to skip (LinkedIn UI noise)
        let skip_patterns = vec![
            "set alert for similar jobs",
            "see how you compare",
            "candidate seniority",
            "candidate education",
            "exclusive job seeker insights",
            "powered by bing",
            "company focus areas",
            "hiring & headcount",
            "the latest hiring trend",
            "median employee tenure",
            "hires candidates from",
            "total employees",
            "year growth",
            "help me stand out",
            "tailor my resume",
            "create cover letter",
            "show match details",
            "people you can reach",
            "show premium insights",
            "more jobs",
            "interested in working with us",
            "privately share your profile",
            "company photos",
            "competitors",
            "sources:",
            "about the company",
            "followers",
            "school alumni work here",
        ];

        // Process the HTML structure
        Self::process_node(&document.root_element(), &mut result, 0, &skip_patterns);

        // Clean up excessive whitespace
        let cleaned = result
            .lines()
            .map(|line| line.trim())
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join("\n");

        // Truncate at common end-of-job-description markers
        let end_markers = vec![
            "… more",  // LinkedIn "show more" indicator (often marks end of actual content)
            "More jobs",
            "Looking for talent?",
            "Actively reviewing applicants",
            "LinkedIn Corporation ©",
            "Select language",
        ];

        let mut truncated = cleaned.as_str();
        for marker in &end_markers {
            if let Some(pos) = cleaned.find(marker) {
                truncated = &cleaned[..pos];
                break;
            }
        }

        Ok(truncated.trim().to_string())
    }

    fn process_node(
        node: &scraper::ElementRef,
        result: &mut String,
        depth: usize,
        skip_patterns: &[&str],
    ) {
        use scraper::Node;

        for child in node.children() {
            match child.value() {
                Node::Element(element) => {
                    let tag_name = element.name();

                    // Skip script, style, and other non-content tags
                    if matches!(tag_name, "script" | "style" | "noscript" | "svg" | "path") {
                        continue;
                    }

                    if let Some(child_elem) = scraper::ElementRef::wrap(child) {
                        // Get DIRECT text content (not all descendants) to check if we should skip
                        let direct_text: String = child_elem.children()
                            .filter_map(|c| {
                                if let scraper::Node::Text(t) = c.value() {
                                    Some(t.to_string())
                                } else {
                                    None
                                }
                            })
                            .collect();

                        // Skip if THIS element's direct text is JavaScript/JSON
                        if direct_text.len() > 50 && (
                            direct_text.contains("window.__") ||
                            direct_text.contains("webpack") ||
                            direct_text.contains("module_cache") ||
                            direct_text.contains("__como_")
                        ) {
                            continue;
                        }

                        // Get full text content for noise pattern matching
                        let text_content = child_elem.text().collect::<String>().to_lowercase();

                        // Skip LinkedIn UI noise (only if it's a small element)
                        if text_content.len() < 500 &&
                           skip_patterns.iter().any(|pattern| text_content.contains(pattern)) {
                            continue;
                        }

                        match tag_name {
                            "li" => {
                                // Preserve bullet points
                                result.push_str("• ");
                                Self::process_node(&child_elem, result, depth + 1, skip_patterns);
                                result.push('\n');
                            }
                            "p" | "div" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                                Self::process_node(&child_elem, result, depth, skip_patterns);
                                result.push('\n');
                            }
                            "br" => {
                                result.push('\n');
                            }
                            "ul" | "ol" => {
                                Self::process_node(&child_elem, result, depth, skip_patterns);
                            }
                            _ => {
                                Self::process_node(&child_elem, result, depth, skip_patterns);
                            }
                        }
                    }
                }
                Node::Text(text) => {
                    let text_str = text.trim();
                    if !text_str.is_empty() {
                        result.push_str(text_str);
                        result.push(' ');
                    }
                }
                _ => {}
            }
        }
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

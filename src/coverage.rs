//! Code coverage collection for JavaScript and CSS.
//!
//! Wraps the CDP `Profiler` and `CSS` domains to provide convenient
//! coverage collection that hides all protocol details.

use crate::cdp::browser_protocol::css::{
    DisableParams as CssDisableParams, EnableParams as CssEnableParams,
    StartRuleUsageTrackingParams, StopRuleUsageTrackingParams,
};
use crate::cdp::js_protocol::profiler::{
    DisableParams as ProfilerDisableParams, EnableParams as ProfilerEnableParams,
    StartPreciseCoverageParams, StopPreciseCoverageParams, TakePreciseCoverageParams,
};
use crate::error::Result;
use crate::target::Target;

/// A range of source code with an execution count.
#[derive(Debug, Clone)]
pub struct CoverageRange {
    pub start_offset: i64,
    pub end_offset: i64,
    pub count: i64,
}

/// JavaScript code coverage entry for a single script.
#[derive(Debug, Clone)]
pub struct JSCoverageEntry {
    pub url: String,
    pub ranges: Vec<CoverageRange>,
}

/// CSS code coverage entry for a single stylesheet rule.
#[derive(Debug, Clone)]
pub struct CSSCoverageEntry {
    pub style_sheet_id: String,
    pub start_offset: f64,
    pub end_offset: f64,
    pub used: bool,
}

/// Manages code coverage collection on a page.
pub struct Coverage {
    target: Target,
}

impl Coverage {
    pub(crate) fn new(target: Target) -> Self {
        Self { target }
    }

    /// Start collecting JavaScript coverage.
    ///
    /// Enables the Profiler domain and begins precise coverage collection
    /// with call counts and detailed (block-level) granularity.
    pub async fn start_js_coverage(&self) -> Result<()> {
        self.target.execute(ProfilerEnableParams::default()).await?;
        self.target
            .execute(StartPreciseCoverageParams {
                call_count: Some(true),
                detailed: Some(true),
                allow_triggered_updates: None,
            })
            .await?;
        Ok(())
    }

    /// Stop JavaScript coverage collection and return results.
    ///
    /// Takes a precise coverage snapshot, then disables the Profiler domain.
    /// Each returned entry represents one script with its coverage ranges.
    pub async fn stop_js_coverage(&self) -> Result<Vec<JSCoverageEntry>> {
        let result = self
            .target
            .execute(TakePreciseCoverageParams::default())
            .await?;

        let entries = result
            .result
            .into_iter()
            .map(|script| {
                let ranges = script
                    .functions
                    .into_iter()
                    .flat_map(|func| func.ranges)
                    .map(|r| CoverageRange {
                        start_offset: r.start_offset,
                        end_offset: r.end_offset,
                        count: r.count,
                    })
                    .collect();
                JSCoverageEntry {
                    url: script.url,
                    ranges,
                }
            })
            .collect();

        self.target
            .execute(StopPreciseCoverageParams::default())
            .await?;
        self.target
            .execute(ProfilerDisableParams::default())
            .await?;

        Ok(entries)
    }

    /// Start collecting CSS coverage.
    ///
    /// Enables the CSS domain and begins rule usage tracking.
    pub async fn start_css_coverage(&self) -> Result<()> {
        self.target.execute(CssEnableParams::default()).await?;
        self.target
            .execute(StartRuleUsageTrackingParams::default())
            .await?;
        Ok(())
    }

    /// Stop CSS coverage collection and return results.
    ///
    /// Stops rule usage tracking and disables the CSS domain.
    /// Each returned entry represents one CSS rule and whether it was used.
    pub async fn stop_css_coverage(&self) -> Result<Vec<CSSCoverageEntry>> {
        let result = self
            .target
            .execute(StopRuleUsageTrackingParams::default())
            .await?;

        let entries = result
            .rule_usage
            .into_iter()
            .map(|rule| CSSCoverageEntry {
                style_sheet_id: rule.style_sheet_id.inner().to_string(),
                start_offset: rule.start_offset,
                end_offset: rule.end_offset,
                used: rule.used,
            })
            .collect();

        self.target.execute(CssDisableParams::default()).await?;

        Ok(entries)
    }
}

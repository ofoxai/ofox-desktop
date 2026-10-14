//! Cost Calculator - 计算 API 请求成本
//!
//! 使用高精度 Decimal 类型避免浮点数精度问题

use super::parser::TokenUsage;
use rust_decimal::Decimal;
use std::str::FromStr;

/// 成本明细
#[derive(Debug, Clone)]
pub struct CostBreakdown {
    pub input_cost: Decimal,
    pub output_cost: Decimal,
    pub cache_read_cost: Decimal,
    pub cache_creation_cost: Decimal,
    pub total_cost: Decimal,
}

/// 模型定价信息
#[derive(Debug, Clone)]
pub struct ModelPricing {
    pub input_cost_per_million: Decimal,
    pub output_cost_per_million: Decimal,
    pub cache_read_cost_per_million: Decimal,
    pub cache_creation_cost_per_million: Decimal,
}

/// 成本计算器
pub struct CostCalculator;

impl CostCalculator {
    /// 计算请求成本
    ///
    /// # 参数
    /// - `usage`: Token 使用量
    /// - `pricing`: 模型定价
    /// - `cost_multiplier`: 成本倍数 (provider 自定义)
    ///
    /// # 计算逻辑
    /// Every source stores `input_tokens` as fresh input: it excludes cache reads
    /// and cache writes (Claude reports it that way; the Codex, Gemini and OpenAI
    /// parsers subtract the cached part before storing). So each bucket is priced
    /// once, nothing is subtracted here, and this is the only cost formula:
    /// - input_cost: input_tokens × 输入价格
    /// - cache_read_cost: cache_read_tokens × 缓存读取价格
    /// - cache_creation_cost: cache_creation_tokens × 缓存写入价格
    /// - total_cost: 各项成本之和 × 倍率（倍率只作用于最终总价）
    pub fn calculate(
        usage: &TokenUsage,
        pricing: &ModelPricing,
        cost_multiplier: Decimal,
    ) -> CostBreakdown {
        let million = Decimal::from(1_000_000);

        // 各项基础成本（不含倍率）
        let input_cost =
            Decimal::from(usage.input_tokens) * pricing.input_cost_per_million / million;
        let output_cost =
            Decimal::from(usage.output_tokens) * pricing.output_cost_per_million / million;
        let cache_read_cost =
            Decimal::from(usage.cache_read_tokens) * pricing.cache_read_cost_per_million / million;
        let cache_creation_cost = Decimal::from(usage.cache_creation_tokens)
            * pricing.cache_creation_cost_per_million
            / million;

        // 总成本 = 各项基础成本之和 × 倍率
        let base_total = input_cost + output_cost + cache_read_cost + cache_creation_cost;
        let total_cost = base_total * cost_multiplier;

        CostBreakdown {
            input_cost,
            output_cost,
            cache_read_cost,
            cache_creation_cost,
            total_cost,
        }
    }

    /// 尝试计算成本，如果模型未知则返回 None
    pub fn try_calculate(
        usage: &TokenUsage,
        pricing: Option<&ModelPricing>,
        cost_multiplier: Decimal,
    ) -> Option<CostBreakdown> {
        pricing.map(|p| Self::calculate(usage, p, cost_multiplier))
    }
}

impl ModelPricing {
    /// 从字符串创建定价信息
    pub fn from_strings(
        input: &str,
        output: &str,
        cache_read: &str,
        cache_creation: &str,
    ) -> Result<Self, rust_decimal::Error> {
        Ok(Self {
            input_cost_per_million: Decimal::from_str(input)?,
            output_cost_per_million: Decimal::from_str(output)?,
            cache_read_cost_per_million: Decimal::from_str(cache_read)?,
            cache_creation_cost_per_million: Decimal::from_str(cache_creation)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(input: u32, output: u32, cache_read: u32, cache_creation: u32) -> TokenUsage {
        TokenUsage {
            input_tokens: input,
            output_tokens: output,
            cache_read_tokens: cache_read,
            cache_creation_tokens: cache_creation,
            model: None,
            message_id: None,
        }
    }

    fn dec(s: &str) -> Decimal {
        Decimal::from_str(s).unwrap()
    }

    /// Every source stores `input_tokens` as fresh input (no cache reads, no cache
    /// writes), so the calculator prices each bucket once and never subtracts.
    #[test]
    fn every_source_prices_fresh_input_and_each_cache_bucket_separately() {
        // $3 in, $15 out, $0.30 cache read, $3.75 cache write (per million)
        let pricing = ModelPricing::from_strings("3.0", "15.0", "0.3", "3.75").unwrap();
        // (source, stored usage, input, output, cache read, cache write costs)
        let cases = [
            // Claude JSONL: input 3 excludes the 5000 read and 10000 written
            (
                "claude session_log",
                usage(3, 150, 5000, 10000),
                "0.000009",
                "0.00225",
                "0.0015",
                "0.0375",
            ),
            // Codex rollout: 1000 raw input with 800 cached is stored as 200 fresh
            (
                "codex_session",
                usage(200, 50, 800, 0),
                "0.0006",
                "0.00075",
                "0.00024",
                "0",
            ),
            // Gemini chat: promptTokenCount 900 with 600 cached is stored as 300
            (
                "gemini_session",
                usage(300, 40, 600, 0),
                "0.0009",
                "0.0006",
                "0.00018",
                "0",
            ),
            // OpenCode: its input already excludes the cache
            (
                "opencode_session",
                usage(40, 60, 700, 300),
                "0.00012",
                "0.0009",
                "0.00021",
                "0.001125",
            ),
            // a proxy row whose cache read exceeds the fresh input is not clamped
            (
                "proxy",
                usage(10, 0, 20, 0),
                "0.00003",
                "0",
                "0.000006",
                "0",
            ),
        ];
        for (source, usage, input, output, cache_read, cache_creation) in cases {
            let cost = CostCalculator::calculate(&usage, &pricing, Decimal::ONE);
            assert_eq!(cost.input_cost, dec(input), "{source}: input cost");
            assert_eq!(cost.output_cost, dec(output), "{source}: output cost");
            assert_eq!(
                cost.cache_read_cost,
                dec(cache_read),
                "{source}: cache read"
            );
            assert_eq!(
                cost.cache_creation_cost,
                dec(cache_creation),
                "{source}: cache write"
            );
            assert_eq!(
                cost.total_cost,
                cost.input_cost
                    + cost.output_cost
                    + cost.cache_read_cost
                    + cost.cache_creation_cost,
                "{source}: total"
            );
        }
    }

    #[test]
    fn claude_fresh_input_is_priced_even_when_cache_reads_dwarf_it() {
        let pricing = ModelPricing::from_strings("3.0", "15.0", "0.3", "3.75").unwrap();
        let cost = CostCalculator::calculate(&usage(3, 150, 5000, 10000), &pricing, Decimal::ONE);
        assert!(cost.input_cost > Decimal::ZERO);
        assert!(cost.cache_read_cost > Decimal::ZERO);
        assert!(cost.cache_creation_cost > Decimal::ZERO);
    }

    #[test]
    fn test_cost_multiplier() {
        let usage = TokenUsage {
            input_tokens: 1000,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            model: None,
            message_id: None,
        };

        let pricing = ModelPricing::from_strings("3.0", "15.0", "0", "0").unwrap();
        let multiplier = Decimal::from_str("1.5").unwrap();

        let cost = CostCalculator::calculate(&usage, &pricing, multiplier);

        // input_cost: 基础价格（不含倍率）= 1000 * 3.0 / 1M = 0.003
        assert_eq!(cost.input_cost, Decimal::from_str("0.003").unwrap());
        // total_cost: 基础价格 × 倍率 = 0.003 * 1.5 = 0.0045
        assert_eq!(cost.total_cost, Decimal::from_str("0.0045").unwrap());
    }

    #[test]
    fn test_unknown_model_handling() {
        let usage = TokenUsage {
            input_tokens: 1000,
            output_tokens: 500,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            model: None,
            message_id: None,
        };

        let multiplier = Decimal::from_str("1.0").unwrap();
        let cost = CostCalculator::try_calculate(&usage, None, multiplier);

        assert!(cost.is_none());
    }

    #[test]
    fn test_decimal_precision() {
        let usage = TokenUsage {
            input_tokens: 1,
            output_tokens: 1,
            cache_read_tokens: 1,
            cache_creation_tokens: 1,
            model: None,
            message_id: None,
        };

        let pricing = ModelPricing::from_strings("0.075", "0.3", "0.01875", "0.075").unwrap();
        let multiplier = Decimal::from_str("1.0").unwrap();

        let cost = CostCalculator::calculate(&usage, &pricing, multiplier);

        // 验证高精度计算
        assert!(cost.total_cost > Decimal::ZERO);
        assert!(cost.total_cost.to_string().len() > 2); // 确保保留了小数位
    }
}

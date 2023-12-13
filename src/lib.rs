#![feature(async_fn_in_trait)]

use anyhow::Result;
use chrono::prelude::*;
use log::info;
use pushover::requests::message::SendMessage;
use reqwest::header;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::env;

pub static CONFIG_FILENAME: &str = "settings.toml";

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub struct Config {
    #[serde(rename = "config_path")]
    pub config_path: String,

    pub pushover_user_key: String,
    pub pushover_api_key: String,

    pub ynab_bearer_token: String,
    pub ynab_budget_id: String,
    pub ynab_reconciliation_payee_id: String,
}

#[derive(Clone, Debug)]
pub struct YnabAccountConfig {
    pub ynab_account_id: String,
}

pub trait GetYnabAccountConfig {
    async fn get(&self) -> Result<YnabAccountConfig>;
}

pub trait GetBalance {
    async fn get(&self) -> Result<f32>;
}

async fn _update_ynab<T>(config: &Config, t: T) -> Result<()>
where
    T: GetBalance + GetYnabAccountConfig,
{
    let ynab_account_config = GetYnabAccountConfig::get(&t).await?;

    let real_balance = GetBalance::get(&t).await?;

    info!("Real Balance: {:#?}", real_balance);

    let mut headers = header::HeaderMap::new();
    headers.insert(
        "Authorization",
        format!("Bearer {}", config.ynab_bearer_token).parse()?,
    );
    headers.insert("Content-Type", "application/json".parse()?);

    let client = reqwest::Client::builder()
        .default_headers(headers)
        .connection_verbose(true)
        .build()?;

    #[derive(Clone, Debug, Serialize, Deserialize)]
    struct Response<T> {
        data: T,
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    struct AccountWrapper {
        account: Account,
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    struct Account {
        id: String,
        balance: i32,
        last_reconciled_at: String,
    }

    let balance = client
        .get(format!(
            "https://api.ynab.com/v1/budgets/{}/accounts/{}",
            config.ynab_budget_id, ynab_account_config.ynab_account_id
        ))
        .send()
        .await?
        .error_for_status()?
        .json::<Response<AccountWrapper>>()
        .await?
        .data
        .account
        .balance;

    info!("YNAB Balance: {:#?}", balance as f32 / 1000.0);

    #[derive(Clone, Debug, Serialize, Deserialize)]
    struct TransactionWrapper<T> {
        transaction: T,
    }
    #[derive(Clone, Debug, Serialize, Deserialize)]
    struct Transactions {
        transactions: Vec<Transaction>,
    }
    #[derive(Clone, Debug, Serialize, Deserialize)]
    struct Transaction {
        id: String,
        #[serde(flatten)]
        transaction: CreateTransaction,
    }
    #[derive(Clone, Debug, Serialize, Deserialize)]
    struct CreateTransaction {
        date: NaiveDate,
        amount: i32,
        payee_id: String,
        #[serde(flatten)]
        other: serde_json::Value,
    }

    let transactions_response = client
        .get(format!(
            "https://api.ynab.com/v1/budgets/{}/accounts/{}/transactions",
            config.ynab_budget_id, ynab_account_config.ynab_account_id
        ))
        .send()
        .await?
        .error_for_status()?
        .json::<Response<Transactions>>()
        .await?;

    let last_transaction = transactions_response
        .data
        .transactions
        .last()
        .unwrap()
        .to_owned();

    let real_balance_milli = { real_balance * 1000.0 } as i32;
    let balance_adjustment = real_balance_milli - balance;

    let now = Local::now().date_naive();

    if balance == real_balance_milli {
        info!("Real & YNAB balances are equal");
        Ok(())
    } else if last_transaction.transaction.date.day() == 1 {
        info!("There's already a transaction for the 1st");
        Ok(())
    } else if last_transaction.transaction.payee_id
        == config.ynab_reconciliation_payee_id
        // preserve the adjustment transaction on the 1st to create a record of the account's value over time
        && last_transaction.transaction.date.day() != 1
    {
        info!("Real & YNAB balances are not equal and the last transaction was a reconciliation");
        let body = TransactionWrapper {
            transaction: Transaction {
                transaction: CreateTransaction {
                    amount: last_transaction.transaction.amount + balance_adjustment,
                    date: now,
                    ..last_transaction.transaction.clone()
                },
                ..last_transaction.clone()
            },
        };
        let response = client
            .put(format!(
                "https://api.ynab.com/v1/budgets/{}/transactions/{}",
                config.ynab_budget_id, last_transaction.id
            ))
            .json(&body)
            .send()
            .await?
            .error_for_status()?;
        info!("PUT response {:#?}", response.status());
        Ok(())
    } else {
        info!(
            "Real & YNAB balances are not equal and the last transaction was not a reconciliation or it's the 1st"
        );
        let body = TransactionWrapper {
            transaction: CreateTransaction {
                amount: balance_adjustment,
                date: now,
                payee_id: config.ynab_reconciliation_payee_id.clone(),
                other: json!({
                    "account_id": ynab_account_config.ynab_account_id,
                    "approved": true,
                    "category_name": "Uncategorized",
                    "cleared": "reconciled",
                    "memo": "Entered automatically by YNAB",
                    "payee_name": "Reconciliation Balance Adjustment"
                }),
            },
        };
        let response = client
            .post(format!(
                "https://api.ynab.com/v1/budgets/{}/transactions",
                config.ynab_budget_id
            ))
            .json(&body)
            .send()
            .await?
            .error_for_status()?;
        info!("POST response {:#?}", response.status());
        Ok(())
    }
}

pub async fn update_ynab<T>(t: T) -> Result<()>
where
    T: GetBalance + GetYnabAccountConfig,
{
    let config_path = format!("{}/{}", env::var("YNAB_CONFIG_PATH")?, CONFIG_FILENAME);

    let config = config::Config::builder()
        .add_source(config::File::with_name(&config_path))
        .add_source(config::Environment::with_prefix("YNAB"))
        .build()?
        .try_deserialize::<Config>()?;

    match _update_ynab(&config, t).await {
        Ok(()) => Ok(()),
        Err(e) => {
            let api = pushover::API::new();
            let msg = SendMessage::new(
                config.pushover_api_key,
                config.pushover_user_key,
                format!("Failed to update YNAB: {:#?}", e.to_string()),
            );
            api.send(&msg).unwrap();
            Err(e)
        }
    }
}

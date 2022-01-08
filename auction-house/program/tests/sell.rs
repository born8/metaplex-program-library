#![cfg(feature = "test-bpf")]
mod utils;
use anchor_lang::{AccountDeserialize, InstructionData, ToAccountMetas};

use mpl_auction_house::{pda::*, AuctionHouse};
use mpl_testing_utils::metadata::Metadata;
use mpl_testing_utils::solana::{airdrop, create_associated_token_account, create_mint};
use solana_program_test::*;
use solana_sdk::{
    instruction::{Instruction, InstructionError},
    sysvar,
    transaction::{Transaction, TransactionError},
    transport::TransportError,
};
use solana_sdk::{signature::Keypair, signer::Signer};
use spl_token;
use std::assert_eq;
use utils::setup_functions::*;

#[tokio::test]
async fn sell_success() {
    let mut context = auction_house_program_test().start_with_context().await;
    // Payer Wallet
    let payer_wallet = Keypair::new();

    airdrop(&mut context, &payer_wallet.pubkey(), 10_000_000_000)
        .await
        .unwrap();
    let (ah, ahkey) = existing_auction_house_test_context(&mut context)
        .await
        .unwrap();

    let test_metadata = Metadata::new();
    test_metadata
        .create(
            &mut context,
            "Test".to_string(),
            "TST".to_string(),
            "uri".to_string(),
            None,
            10,
            false,
        )
        .await
        .unwrap();
    let price = 1;
    let size = 1;
    let (seller_trade_state, sts_bump) = find_trade_state_address(
        &payer_wallet.pubkey().clone(),
        &ahkey,
        &payer_wallet.pubkey().clone(),
        &ah.treasury_mint,
        &test_metadata.mint.pubkey(),
        price,
        size,
    );
    let (free_seller_trade_state, free_sts_bump) = find_trade_state_address(
        &payer_wallet.pubkey().clone(),
        &ahkey,
        &payer_wallet.pubkey().clone(),
        &ah.treasury_mint,
        &test_metadata.mint.pubkey(),
        0,
        size,
    );
    let (pas, pas_bump) = find_program_as_signer_address();
    let accounts = mpl_auction_house::accounts::Sell {
        wallet: payer_wallet.pubkey().clone(),
        token_account: payer_wallet.pubkey().clone(),
        metadata: test_metadata.pubkey,
        authority: ah.authority,
        auction_house: ahkey,
        auction_house_fee_account: ah.auction_house_fee_account,
        seller_trade_state,
        free_seller_trade_state: free_seller_trade_state,
        token_program: spl_token::id(),
        system_program: solana_program::system_program::id(),
        program_as_signer: pas,
        rent: sysvar::rent::id(),
    }
    .to_account_metas(None);

    let data = mpl_auction_house::instruction::Sell {
        trade_state_bump: sts_bump,
        _free_trade_state_bump: free_sts_bump,
        _program_as_signer_bump: pas_bump,
        token_size: size,
        buyer_price: price,
    }
    .data();

    let instruction = Instruction {
        program_id: mpl_auction_house::id(),
        data,
        accounts,
    };

    let tx = Transaction::new_signed_with_payer(
        &[instruction],
        Some(&payer_wallet.pubkey()),
        &[&payer_wallet],
        context.last_blockhash,
    );

    context.banks_client.process_transaction(tx).await.unwrap();
    let sts = context
        .banks_client
        .get_account(seller_trade_state)
        .await
        .expect("Error Getting Trade State")
        .expect("Trade State Empty");
    assert!(sts.data.len() == 1);
}

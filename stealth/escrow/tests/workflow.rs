use {
    anchor_lang::{
        InstructionData,
        ToAccountMetas,
    },
    stealth::{
        encryption::elgamal::{
            ElGamalCiphertext,
            ElGamalKeypair,
            CipherKey,
        },
        instruction::get_stealth_address,
        pod::PodAccountInfo,
        state::StealthAccount,
    },
    rand_core::OsRng,
    solana_program_test::*,
    solana_sdk::{
        instruction::{AccountMeta, Instruction},
        native_token::LAMPORTS_PER_SOL,
        program_pack::Pack,
        pubkey::Pubkey,
        signer::keypair::Keypair,
        signature::Signer,
        system_instruction,
        system_program,
        transaction::Transaction,
    },
    std::convert::TryInto,
};

async fn nft_setup_transaction(
    payer: &dyn Signer,
    mint: &dyn Signer,
    recent_blockhash: &solana_sdk::hash::Hash,
    rent: &solana_sdk::sysvar::rent::Rent,
    elgamal_kp: &ElGamalKeypair,
    cipher_key: &CipherKey,
) -> Result<(Transaction, Pubkey), Box<dyn std::error::Error>> {
    let (public_metadata_key, _public_metadata_bump) = Pubkey::find_program_address(
        &[
            mpl_token_metadata::state::PREFIX.as_bytes(),
            mpl_token_metadata::id().as_ref(),
            mint.pubkey().as_ref(),
        ],
        &mpl_token_metadata::id(),
    );

    let (public_edition_key, _public_edition_bump) = Pubkey::find_program_address(
        &[
            mpl_token_metadata::state::PREFIX.as_bytes(),
            mpl_token_metadata::id().as_ref(),
            mint.pubkey().as_ref(),
            mpl_token_metadata::state::EDITION.as_bytes(),
        ],
        &mpl_token_metadata::id(),
    );

    let payer_pubkey = payer.pubkey();
    let instructions = vec![
            system_instruction::create_account(
                &payer.pubkey(),
                &mint.pubkey(),
                rent.minimum_balance(spl_token::state::Mint::LEN),
                spl_token::state::Mint::LEN as u64,
                &spl_token::id(),
            ),
            spl_token::instruction::initialize_mint(
                &spl_token::id(),
                &mint.pubkey(),
                &payer.pubkey(), // mint auth
                Some(&payer_pubkey), // freeze auth
                0,
            )?,
            spl_associated_token_account::create_associated_token_account(
                &payer.pubkey(), // funding
                &payer.pubkey(), // wallet to create for
                &mint.pubkey(),
            ),
            spl_token::instruction::mint_to(
                &spl_token::id(),
                &mint.pubkey(),
                &spl_associated_token_account::get_associated_token_address(
                    &payer.pubkey(),
                    &mint.pubkey(),
                ),
                &payer.pubkey(),
                &[],
                1
            )?,
            spl_token::instruction::approve(
                &spl_token::id(),
                &spl_associated_token_account::get_associated_token_address(
                    &payer.pubkey(),
                    &mint.pubkey(),
                ),
                &get_stealth_address(&mint.pubkey()).0, // delegate
                &payer.pubkey(), // owner
                &[],
                1,
            )?,
            mpl_token_metadata::instruction::create_metadata_accounts(
                mpl_token_metadata::id(),
                public_metadata_key,
                mint.pubkey(),
                payer.pubkey(), // mint auth
                payer.pubkey(), // payer
                payer.pubkey(), // update auth
                "test".to_string(), // name
                "".to_string(), // symbol
                "".to_string(), // uri
                Some(vec![mpl_token_metadata::state::Creator{
                    address: payer.pubkey(),
                    verified: true,
                    share: 100,
                }]),
                0, // seller_fee_basis_points
                true, // update_auth_is_signer
                true, // is_mutable
            ),
            mpl_token_metadata::instruction::create_master_edition(
                mpl_token_metadata::id(),
                public_edition_key,
                mint.pubkey(),
                payer.pubkey(), // update auth
                payer.pubkey(), // mint auth
                public_metadata_key,
                payer.pubkey(), // payer
                None, // limited edition supply
            ),
            stealth::instruction::configure_metadata(
                payer.pubkey(),
                mint.pubkey(),
                elgamal_kp.public.into(),
                &elgamal_kp.public.encrypt(*cipher_key).into(),
                &[],
            ),
        ];

    Ok((Transaction::new_signed_with_payer(
        &instructions,
        Some(&payer.pubkey()),
        &[payer, mint],
        *recent_blockhash,
    ), public_metadata_key))
}

#[tokio::test]
async fn test_successful_escrow() {
    let mut pc = ProgramTest::default();

    pc.prefer_bpf(true);

    pc.add_program("mpl_token_metadata", mpl_token_metadata::id(), None);
    pc.add_program("stealth", stealth::id(), None);
    pc.add_program("stealth_escrow", stealth_escrow::id(), None);

    pc.set_compute_max_units(20_000_000);

    let (mut banks_client, payer, recent_blockhash) = pc.start().await;
    let rent = banks_client.get_rent().await;
    let rent = rent.unwrap();

    let mint = Keypair::from_base58_string("47WBGggARowPAzDVdCMCGxTVhNBqXhxgyDcFFyGrVx3VqUyPU7UZTz9umQifQA8yXxKNX8sKGujtDKu7kKX1rLB8");

    let elgamal_kp = ElGamalKeypair::new(&payer, &mint.pubkey()).unwrap();
    let cipher_key = CipherKey::random(&mut OsRng);

    println!("mint {:?}", mint);

    // smoke test
    assert_eq!(
        elgamal_kp.public.encrypt(cipher_key).decrypt(&elgamal_kp.secret),
        Ok(cipher_key),
    );

    let (nft_setup, public_metadata_key) = nft_setup_transaction(
        &payer,
        &mint,
        &recent_blockhash,
        &rent,
        &elgamal_kp,
        &cipher_key,
    ).await.unwrap();

    banks_client.process_transaction(nft_setup).await.unwrap();

    // data landed...
    let stealth_key = get_stealth_address(&mint.pubkey()).0;
    let stealth_account = banks_client.get_account(stealth_key).await.unwrap().unwrap();
    let stealth = StealthAccount::from_bytes(stealth_account.data.as_slice()).unwrap();
    assert_eq!(
        stealth.encrypted_cipher_key.try_into().and_then(
            |ct: ElGamalCiphertext| ct.decrypt(&elgamal_kp.secret)),
        Ok(cipher_key),
    );

    let seller = &payer;
    let buyer = Keypair::new();

    // seed buyer
    banks_client.process_transaction(
        Transaction::new_signed_with_payer(
            &[
                system_instruction::transfer(
                    &seller.pubkey(),
                    &buyer.pubkey(),
                    LAMPORTS_PER_SOL,
                ),
            ],
            Some(&seller.pubkey()),
            &[seller],
            recent_blockhash,
        ),
    ).await.unwrap();

    let (escrow_key, _escrow_bump) = Pubkey::find_program_address(
        &[
            b"BidEscrow",
            buyer.pubkey().as_ref(),
            mint.pubkey().as_ref(),
        ],
        &stealth_escrow::ID,
    );

    let buyer_elgamal_kp = ElGamalKeypair::new(&buyer, &mint.pubkey()).unwrap();
    let transfer_buffer_key = stealth::instruction::get_transfer_buffer_address(
        &buyer.pubkey(), &mint.pubkey()).0;

    // buyer makes a bid (of 0 haha...) and publishes elgamal pubkey
    banks_client.process_transaction(
        Transaction::new_signed_with_payer(
            &[
                Instruction {
                    program_id: stealth_escrow::id(),
                    data: stealth_escrow::instruction::InitEscrow {
                        collateral: LAMPORTS_PER_SOL,
                        slots: 1000,
                    }.data(),
                    accounts: stealth_escrow::accounts::InitEscrow {
                        bidder: buyer.pubkey(),
                        mint: mint.pubkey(),
                        escrow: escrow_key,
                        system_program: system_program::id(),
                    }.to_account_metas(None),
                },
                stealth::instruction::publish_elgamal_pubkey(
                    &buyer.pubkey(),
                    &mint.pubkey(),
                    buyer_elgamal_kp.public.into(),
                ),
            ],
            Some(&buyer.pubkey()),
            &[&buyer],
            recent_blockhash,
        ),
    ).await.unwrap();

    // seller accepts
    banks_client.process_transaction(
        Transaction::new_signed_with_payer(
            &[
                stealth_escrow::accept_escrow(
                    buyer.pubkey(),
                    mint.pubkey(),
                    escrow_key,
                    seller.pubkey(),
                ),
            ],
            Some(&seller.pubkey()),
            &[seller],
            recent_blockhash,
        ),
    ).await.unwrap();

    // crank over 'many' transactions
    banks_client.process_transaction(
        Transaction::new_signed_with_payer(
            &[
                stealth::instruction::transfer_chunk(
                    seller.pubkey(),
                    mint.pubkey(),
                    transfer_buffer_key,
                    stealth::instruction::TransferChunkData {
                        transfer: stealth::transfer_proof::TransferData::new(
                            &elgamal_kp,
                            buyer_elgamal_kp.public.into(),
                            cipher_key,
                            stealth.encrypted_cipher_key.try_into().unwrap(),
                        ),
                    },
                ),
            ],
            Some(&seller.pubkey()),
            &[seller],
            recent_blockhash,
        ),
    ).await.unwrap();

    // and complete escrow
    let mut complete_escrow_accounts = stealth_escrow::accounts::CompleteEscrow {
        bidder: buyer.pubkey(),
        mint: mint.pubkey(),
        escrow: escrow_key,
        bidder_token_account:
            spl_associated_token_account::get_associated_token_address(
                &buyer.pubkey(),
                &mint.pubkey(),
            ),
        acceptor: seller.pubkey(),
        escrow_token_account:
            spl_associated_token_account::get_associated_token_address(
                &escrow_key,
                &mint.pubkey(),
            ),
        stealth: stealth_key,
        transfer_buffer: transfer_buffer_key,
        metadata: public_metadata_key,
        system_program: system_program::id(),
        token_program: spl_token::id(),
        stealth_program: stealth::id(),
        rent: solana_sdk::sysvar::rent::id(),
    }.to_account_metas(None);
    complete_escrow_accounts.push(
        AccountMeta::new_readonly(seller.pubkey(), false),
    );
    banks_client.process_transaction(
        Transaction::new_signed_with_payer(
            &[
                // TODO: do conditionally in complete_escrow?
                spl_associated_token_account::create_associated_token_account(
                    &seller.pubkey(), // funding
                    &buyer.pubkey(), // wallet to create for
                    &mint.pubkey(),
                ),
                Instruction {
                    program_id: stealth_escrow::id(),
                    data: stealth_escrow::instruction::CompleteEscrow {}.data(),
                    accounts: complete_escrow_accounts,
                },
            ],
            Some(&seller.pubkey()),
            &[seller],
            recent_blockhash,
        ),
    ).await.unwrap();

    // transfer landed...
    let stealth_account = banks_client.get_account(
        get_stealth_address(&mint.pubkey()).0).await.unwrap().unwrap();
    let stealth = StealthAccount::from_bytes(
        stealth_account.data.as_slice()).unwrap();
    // successfully decrypt with buyer_elgamal_kp
    assert_eq!(
        stealth.encrypted_cipher_key.try_into().and_then(
            |ct: ElGamalCiphertext| ct.decrypt(&buyer_elgamal_kp.secret)),
        Ok(cipher_key),
    );
}
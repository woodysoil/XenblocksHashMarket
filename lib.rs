use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};
use std::mem::size_of;

declare_id!("");

#[program]
pub mod xen_blocks_hash_market {
    use super::*;

    // Initialize the program
    pub fn initialize(ctx: Context<Initialize>) -> Result<()> {
        let market_state = &mut ctx.accounts.market_state;
        market_state.authority = ctx.accounts.authority.key();
        market_state.order_counter = 0;
        market_state.deposit_percentage = 21; // 21% deposit requirement
        market_state.fee_percentage = 500; // 5% Platform fee
        market_state.fee_recipient = ctx.accounts.authority.key(); // Set fee recipient to authority initially

        // Set up tiered fee structure for sellers
        market_state.tier1_volume = 10000 * 10u64.pow(6); // 10,000 USDC
        market_state.tier1_fee = 360; // 3.6%

        market_state.tier2_volume = 50000 * 10u64.pow(6); // 50,000 USDC
        market_state.tier2_fee = 270; // 2.7%

        market_state.tier3_volume = 100000 * 10u64.pow(6); // 100,000 USDC
        market_state.tier3_fee = 200; // 2.0%

        // Minimum order value
        market_state.min_order_value = 10 * 10u64.pow(6); // 10 USDC

        Ok(())
    }

    // Update platform parameters (admin only)
    pub fn update_params(
        ctx: Context<UpdateParams>,
        deposit_percentage: Option<u8>,
        fee_percentage: Option<u16>,
        fee_recipient: Option<Pubkey>,
        tier1_volume: Option<u64>,
        tier1_fee: Option<u16>,
        tier2_volume: Option<u64>,
        tier2_fee: Option<u16>,
        tier3_volume: Option<u64>,
        tier3_fee: Option<u16>,
        min_order_value: Option<u64>,
    ) -> Result<()> {
        let market_state = &mut ctx.accounts.market_state;

        require!(
            ctx.accounts.authority.key() == market_state.authority,
            ErrorCode::Unauthorized
        );

        // Update parameters if provided
        if let Some(value) = deposit_percentage {
            market_state.deposit_percentage = value;
        }

        if let Some(value) = fee_percentage {
            market_state.fee_percentage = value;
        }

        if let Some(value) = fee_recipient {
            market_state.fee_recipient = value;
        }

        if let Some(value) = tier1_volume {
            market_state.tier1_volume = value;
        }

        if let Some(value) = tier1_fee {
            market_state.tier1_fee = value;
        }

        if let Some(value) = tier2_volume {
            market_state.tier2_volume = value;
        }

        if let Some(value) = tier2_fee {
            market_state.tier2_fee = value;
        }

        if let Some(value) = tier3_volume {
            market_state.tier3_volume = value;
        }

        if let Some(value) = tier3_fee {
            market_state.tier3_fee = value;
        }

        if let Some(value) = min_order_value {
            market_state.min_order_value = value;
        }

        Ok(())
    }

    // Create a buy order
    pub fn create_buy_order(
        ctx: Context<CreateBuyOrder>,
        xnm_amount: u64,
        price: u64,
        deadline_days: u8,
        eth_address: String,
    ) -> Result<()> {
        // Validate parameters
        require!(xnm_amount > 0, ErrorCode::InvalidAmount);
        require!(price > 0, ErrorCode::InvalidPrice);
        require!(
            deadline_days > 0 && deadline_days <= 180,
            ErrorCode::InvalidDeadline
        );
        require!(eth_address.len() > 0, ErrorCode::InvalidEthAddress);

        // Validate ETH address format
        require!(
            eth_address.starts_with("0x") && eth_address.len() == 42,
            ErrorCode::InvalidEthAddress
        );

        let market_state = &mut ctx.accounts.market_state;

        // Calculate total order value in USDC
        let total_order_value = xnm_amount
            .checked_mul(price)
            .ok_or(ErrorCode::MathOverflow)?;

        // Check minimum order value
        require!(
            total_order_value >= market_state.min_order_value,
            ErrorCode::OrderTooSmall
        );

        let order_id = market_state.order_counter;
        market_state.order_counter += 1;

        // Transfer USDC from buyer to escrow
        let transfer_instruction = Transfer {
            from: ctx.accounts.buyer_token_account.to_account_info(),
            to: ctx.accounts.escrow_token_account.to_account_info(),
            authority: ctx.accounts.buyer.to_account_info(),
        };

        let cpi_ctx = CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            transfer_instruction,
        );

        token::transfer(cpi_ctx, total_order_value)?;

        // Create and initialize the order account
        let order = &mut ctx.accounts.order;
        order.id = order_id;
        order.order_type = OrderType::Buy;
        order.buyer = ctx.accounts.buyer.key();
        order.seller = None; // Will be filled when accepted
        order.xnm_amount = xnm_amount;
        order.price = price;
        order.created_at = Clock::get()?.unix_timestamp;
        order.completion_days = deadline_days;
        order.deadline = 0; // Will be filled when accepted
        order.eth_address = eth_address;
        order.status = OrderStatus::Open;
        order.total_value = total_order_value;
        order.deposit_amount = 0; // Seller's deposit, will be filled when accepted
        order.completion_percentage = 0; // Not started yet

        Ok(())
    }

    // Create a sell order (for sellers offering mining services)
    pub fn create_sell_order(
        ctx: Context<CreateSellOrder>,
        min_xnm_amount: u64,
        max_xnm_amount: u64,
        price: u64,
        days_to_complete: u8,
    ) -> Result<()> {
        // Validate parameters
        require!(min_xnm_amount > 0, ErrorCode::InvalidAmount);
        require!(max_xnm_amount >= min_xnm_amount, ErrorCode::InvalidAmount);
        require!(price > 0, ErrorCode::InvalidPrice);
        require!(
            days_to_complete > 0 && days_to_complete <= 180,
            ErrorCode::InvalidDeadline
        );

        let market_state = &mut ctx.accounts.market_state;

        // Calculate minimum order value based on min_xnm_amount
        let min_order_value = min_xnm_amount
            .checked_mul(price)
            .ok_or(ErrorCode::MathOverflow)?;

        // Ensure minimum order size
        require!(
            min_order_value >= market_state.min_order_value,
            ErrorCode::OrderTooSmall
        );

        // Calculate deposit based on max potential order value
        let max_order_value = max_xnm_amount
            .checked_mul(price)
            .ok_or(ErrorCode::MathOverflow)?;

        let deposit_percentage = market_state.deposit_percentage as u64;
        let deposit_amount = max_order_value
            .checked_mul(deposit_percentage)
            .ok_or(ErrorCode::MathOverflow)?
            .checked_div(100)
            .ok_or(ErrorCode::MathOverflow)?;

        // Transfer deposit from seller to escrow
        let transfer_instruction = Transfer {
            from: ctx.accounts.seller_token_account.to_account_info(),
            to: ctx.accounts.escrow_token_account.to_account_info(),
            authority: ctx.accounts.seller.to_account_info(),
        };

        let cpi_ctx = CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            transfer_instruction,
        );

        token::transfer(cpi_ctx, deposit_amount)?;

        let order_id = market_state.order_counter;
        market_state.order_counter += 1;

        // Create and initialize the order account
        let order = &mut ctx.accounts.order;
        order.id = order_id;
        order.order_type = OrderType::Sell;
        order.buyer = None; // Will be filled when a buyer takes the order
        order.seller = Some(ctx.accounts.seller.key());
        order.min_xnm_amount = min_xnm_amount;
        order.max_xnm_amount = max_xnm_amount;
        order.xnm_amount = 0; // Will be set when a buyer takes the order
        order.price = price;
        order.created_at = Clock::get()?.unix_timestamp;
        order.completion_days = days_to_complete;
        order.deadline = 0; // Will be set when a buyer takes the order
        order.eth_address = String::new(); // Will be set when a buyer takes the order
        order.status = OrderStatus::Open;
        order.total_value = 0; // Will be set when a buyer takes the order
        order.deposit_amount = deposit_amount;
        order.completion_percentage = 0;

        // Update seller's lifetime volume stats
        let seller_stats = &mut ctx.accounts.seller_stats;
        if !seller_stats.is_initialized {
            seller_stats.seller = ctx.accounts.seller.key();
            seller_stats.lifetime_volume = 0;
            seller_stats.active_orders = 0;
            seller_stats.completed_orders = 0;
            seller_stats.is_initialized = true;
        }

        Ok(())
    }

    // Take a sell order (for buyers)
    pub fn take_sell_order(
        ctx: Context<TakeSellOrder>,
        order_id: u64,
        xnm_amount: u64,
        eth_address: String,
    ) -> Result<()> {
        let market_state = &ctx.accounts.market_state;
        let order = &mut ctx.accounts.order;

        // Validate order
        require!(order.id == order_id, ErrorCode::InvalidOrderId);
        require!(order.status == OrderStatus::Open, ErrorCode::OrderNotOpen);
        require!(
            order.order_type == OrderType::Sell,
            ErrorCode::WrongOrderType
        );

        // Validate amount is within seller's specified range
        require!(
            xnm_amount >= order.min_xnm_amount && xnm_amount <= order.max_xnm_amount,
            ErrorCode::InvalidAmount
        );

        // Validate ETH address
        require!(
            eth_address.starts_with("0x") && eth_address.len() == 42,
            ErrorCode::InvalidEthAddress
        );

        // Calculate required payment
        let total_payment = xnm_amount
            .checked_mul(order.price)
            .ok_or(ErrorCode::MathOverflow)?;

        // Check minimum order value
        require!(
            total_payment >= market_state.min_order_value,
            ErrorCode::OrderTooSmall
        );

        // Calculate actual deposit needed based on order size proportion
        let proportion = (xnm_amount as f64) / (order.max_xnm_amount as f64);
        let required_deposit = (order.deposit_amount as f64 * proportion) as u64;
        let deposit_to_return = order.deposit_amount.saturating_sub(required_deposit);

        // Transfer payment from buyer to escrow
        let transfer_instruction = Transfer {
            from: ctx.accounts.buyer_token_account.to_account_info(),
            to: ctx.accounts.escrow_token_account.to_account_info(),
            authority: ctx.accounts.buyer.to_account_info(),
        };

        let cpi_ctx = CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            transfer_instruction,
        );

        token::transfer(cpi_ctx, total_payment)?;

        // If not using the full capacity of the sell order, return the extra deposit
        if deposit_to_return > 0 {
            let return_instruction = Transfer {
                from: ctx.accounts.escrow_token_account.to_account_info(),
                to: ctx.accounts.seller_token_account.to_account_info(),
                authority: ctx.accounts.market_authority.to_account_info(),
            };

            let market_state_key = market_state.key();
            let seeds = &[market_state_key.as_ref(), &[ctx.bumps.market_state]];
            let signer = &[&seeds[..]];

            let cpi_ctx_return = CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                return_instruction,
                signer,
            );

            token::transfer(cpi_ctx_return, deposit_to_return)?;
        }

        // Update order
        order.buyer = ctx.accounts.buyer.key();
        order.xnm_amount = xnm_amount;
        order.eth_address = eth_address;
        order.status = OrderStatus::InProgress;
        order.total_value = total_payment;
        order.deposit_amount = required_deposit;
        order.deadline = Clock::get()?.unix_timestamp + (order.completion_days as i64 * 86400);

        // Update seller stats
        let seller_stats = &mut ctx.accounts.seller_stats;
        seller_stats.active_orders += 1;

        Ok(())
    }

    // Accept a buy order (for sellers)
    pub fn accept_buy_order(ctx: Context<AcceptBuyOrder>, order_id: u64) -> Result<()> {
        let market_state = &ctx.accounts.market_state;
        let order = &mut ctx.accounts.order;

        // Validate order
        require!(order.id == order_id, ErrorCode::InvalidOrderId);
        require!(order.status == OrderStatus::Open, ErrorCode::OrderNotOpen);
        require!(
            order.order_type == OrderType::Buy,
            ErrorCode::WrongOrderType
        );

        // Calculate required deposit
        let deposit_percentage = market_state.deposit_percentage as u64;
        let deposit_amount = order
            .total_value
            .checked_mul(deposit_percentage)
            .ok_or(ErrorCode::MathOverflow)?
            .checked_div(100)
            .ok_or(ErrorCode::MathOverflow)?;

        // Transfer deposit from seller to escrow
        let transfer_instruction = Transfer {
            from: ctx.accounts.seller_token_account.to_account_info(),
            to: ctx.accounts.escrow_token_account.to_account_info(),
            authority: ctx.accounts.seller.to_account_info(),
        };

        let cpi_ctx = CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            transfer_instruction,
        );

        token::transfer(cpi_ctx, deposit_amount)?;

        // Update order
        order.seller = Some(ctx.accounts.seller.key());
        order.status = OrderStatus::InProgress;
        order.deposit_amount = deposit_amount;
        order.deadline = Clock::get()?.unix_timestamp + (order.completion_days as i64 * 86400);

        // Update seller stats
        let seller_stats = &mut ctx.accounts.seller_stats;
        if !seller_stats.is_initialized {
            seller_stats.seller = ctx.accounts.seller.key();
            seller_stats.lifetime_volume = 0;
            seller_stats.active_orders = 1;
            seller_stats.completed_orders = 0;
            seller_stats.is_initialized = true;
        } else {
            seller_stats.active_orders += 1;
        }

        Ok(())
    }

    // Complete order (when seller has delivered XNM)
    pub fn complete_order(ctx: Context<CompleteOrder>, order_id: u64) -> Result<()> {
        let market_state = &ctx.accounts.market_state;
        let order = &mut ctx.accounts.order;

        // Validate order
        require!(order.id == order_id, ErrorCode::InvalidOrderId);
        require!(
            order.status == OrderStatus::InProgress,
            ErrorCode::InvalidOrderStatus
        );
        require!(
            order.buyer == ctx.accounts.buyer.key(),
            ErrorCode::Unauthorized
        );

        // Calculate fee amount based on seller's tier
        let seller_stats = &mut ctx.accounts.seller_stats;
        let fee_percentage = get_seller_fee_percentage(market_state, seller_stats.lifetime_volume);

        // Calculate fee amount
        let fee_amount = order
            .total_value
            .checked_mul(fee_percentage as u64)
            .ok_or(ErrorCode::MathOverflow)?
            .checked_div(10000) // Fee is in basis points
            .ok_or(ErrorCode::MathOverflow)?;

        // Calculate payment to seller
        let seller_payment = order
            .total_value
            .checked_sub(fee_amount)
            .ok_or(ErrorCode::MathOverflow)?
            .checked_add(order.deposit_amount) // Return deposit to seller
            .ok_or(ErrorCode::MathOverflow)?;

        // Transfer fee to fee recipient
        if fee_amount > 0 {
            let fee_instruction = Transfer {
                from: ctx.accounts.escrow_token_account.to_account_info(),
                to: ctx.accounts.fee_recipient_token_account.to_account_info(),
                authority: ctx.accounts.market_authority.to_account_info(),
            };

            let market_state_key = market_state.key();
            let seeds = &[market_state_key.as_ref(), &[ctx.bumps.market_state]];
            let signer = &[&seeds[..]];

            let cpi_ctx_fee = CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                fee_instruction,
                signer,
            );

            token::transfer(cpi_ctx_fee, fee_amount)?;
        }

        // Transfer payment to seller
        let transfer_instruction = Transfer {
            from: ctx.accounts.escrow_token_account.to_account_info(),
            to: ctx.accounts.seller_token_account.to_account_info(),
            authority: ctx.accounts.market_authority.to_account_info(),
        };

        let market_state_key = market_state.key();
        let seeds = &[market_state_key.as_ref(), &[ctx.bumps.market_state]];
        let signer = &[&seeds[..]];

        let cpi_ctx = CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            transfer_instruction,
            signer,
        );

        token::transfer(cpi_ctx, seller_payment)?;

        // Update order status
        order.status = OrderStatus::Completed;
        order.completion_percentage = 100;

        // Update seller stats
        seller_stats.lifetime_volume = seller_stats
            .lifetime_volume
            .checked_add(order.total_value)
            .ok_or(ErrorCode::MathOverflow)?;
        seller_stats.active_orders = seller_stats.active_orders.saturating_sub(1);
        seller_stats.completed_orders += 1;

        Ok(())
    }

    // Cancel order (only for open orders)
    pub fn cancel_order(ctx: Context<CancelOrder>, order_id: u64) -> Result<()> {
        let market_state = &ctx.accounts.market_state;
        let order = &mut ctx.accounts.order;

        // Validate order
        require!(order.id == order_id, ErrorCode::InvalidOrderId);
        require!(
            order.status == OrderStatus::Open,
            ErrorCode::OrderNotCancellable
        );

        // Check if caller is authorized
        match order.order_type {
            OrderType::Buy => {
                require!(
                    order.buyer == ctx.accounts.caller.key(),
                    ErrorCode::Unauthorized
                );

                // Return funds to buyer
                let transfer_instruction = Transfer {
                    from: ctx.accounts.escrow_token_account.to_account_info(),
                    to: ctx.accounts.user_token_account.to_account_info(),
                    authority: ctx.accounts.market_authority.to_account_info(),
                };

                let market_state_key = market_state.key();
                let seeds = &[market_state_key.as_ref(), &[ctx.bumps.market_state]];
                let signer = &[&seeds[..]];

                let cpi_ctx = CpiContext::new_with_signer(
                    ctx.accounts.token_program.to_account_info(),
                    transfer_instruction,
                    signer,
                );

                token::transfer(cpi_ctx, order.total_value)?;
            }
            OrderType::Sell => {
                require!(
                    order.seller.unwrap() == ctx.accounts.caller.key(),
                    ErrorCode::Unauthorized
                );

                // Return deposit to seller
                let transfer_instruction = Transfer {
                    from: ctx.accounts.escrow_token_account.to_account_info(),
                    to: ctx.accounts.user_token_account.to_account_info(),
                    authority: ctx.accounts.market_authority.to_account_info(),
                };

                let market_state_key = market_state.key();
                let seeds = &[market_state_key.as_ref(), &[ctx.bumps.market_state]];
                let signer = &[&seeds[..]];

                let cpi_ctx = CpiContext::new_with_signer(
                    ctx.accounts.token_program.to_account_info(),
                    transfer_instruction,
                    signer,
                );

                token::transfer(cpi_ctx, order.deposit_amount)?;
            }
        }

        // Update order status
        order.status = OrderStatus::Cancelled;

        Ok(())
    }

    // Partial completion of order (admin resolution)
    pub fn resolve_dispute(
        ctx: Context<ResolveDispute>,
        order_id: u64,
        completion_percentage: u8,
    ) -> Result<()> {
        let market_state = &ctx.accounts.market_state;
        let order = &mut ctx.accounts.order;

        // Validate caller is admin
        require!(
            ctx.accounts.authority.key() == market_state.authority,
            ErrorCode::Unauthorized
        );

        // Validate order
        require!(order.id == order_id, ErrorCode::InvalidOrderId);
        require!(
            order.status == OrderStatus::InProgress,
            ErrorCode::InvalidOrderStatus
        );

        // Validate percentages
        require!(completion_percentage <= 100, ErrorCode::InvalidPercentage);

        // Calculate amounts
        let payment_to_seller = order
            .total_value
            .checked_mul(completion_percentage as u64)
            .ok_or(ErrorCode::MathOverflow)?
            .checked_div(100)
            .ok_or(ErrorCode::MathOverflow)?;

        let refund_to_buyer = order
            .total_value
            .checked_sub(payment_to_seller)
            .ok_or(ErrorCode::MathOverflow)?;

        // Calculate deposit forfeiture
        let deposit_to_forfeit = order
            .deposit_amount
            .checked_mul((100 - completion_percentage) as u64)
            .ok_or(ErrorCode::MathOverflow)?
            .checked_div(100)
            .ok_or(ErrorCode::MathOverflow)?;

        let deposit_to_return = order
            .deposit_amount
            .checked_sub(deposit_to_forfeit)
            .ok_or(ErrorCode::MathOverflow)?;

        // Split forfeited deposit between platform and buyer
        let platform_fee = deposit_to_forfeit
            .checked_mul(50)
            .ok_or(ErrorCode::MathOverflow)?
            .checked_div(100)
            .ok_or(ErrorCode::MathOverflow)?;

        let buyer_compensation = deposit_to_forfeit
            .checked_sub(platform_fee)
            .ok_or(ErrorCode::MathOverflow)?;

        // Calculate regular platform fee on the completed portion
        let seller_stats = &mut ctx.accounts.seller_stats;
        let regular_fee_percentage =
            get_seller_fee_percentage(market_state, seller_stats.lifetime_volume);

        let regular_fee = payment_to_seller
            .checked_mul(regular_fee_percentage as u64)
            .ok_or(ErrorCode::MathOverflow)?
            .checked_div(10000) // Fee is in basis points
            .ok_or(ErrorCode::MathOverflow)?;

        let total_platform_fee = regular_fee
            .checked_add(platform_fee)
            .ok_or(ErrorCode::MathOverflow)?;

        let seller_payment = payment_to_seller
            .checked_sub(regular_fee)
            .ok_or(ErrorCode::MathOverflow)?
            .checked_add(deposit_to_return)
            .ok_or(ErrorCode::MathOverflow)?;

        // Update buyer's refund to include compensation
        let total_buyer_refund = refund_to_buyer
            .checked_add(buyer_compensation)
            .ok_or(ErrorCode::MathOverflow)?;

        // Transfer payments
        let market_state_key = market_state.key();
        let seeds = &[market_state_key.as_ref(), &[ctx.bumps.market_state]];
        let signer = &[&seeds[..]];

        // 1. Transfer platform fee
        if total_platform_fee > 0 {
            let fee_instruction = Transfer {
                from: ctx.accounts.escrow_token_account.to_account_info(),
                to: ctx.accounts.fee_recipient_token_account.to_account_info(),
                authority: ctx.accounts.market_authority.to_account_info(),
            };

            let cpi_ctx_fee = CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                fee_instruction,
                signer,
            );

            token::transfer(cpi_ctx_fee, total_platform_fee)?;
        }

        // 2. Transfer payment to seller
        if seller_payment > 0 {
            let seller_instruction = Transfer {
                from: ctx.accounts.escrow_token_account.to_account_info(),
                to: ctx.accounts.seller_token_account.to_account_info(),
                authority: ctx.accounts.market_authority.to_account_info(),
            };

            let cpi_ctx_seller = CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                seller_instruction,
                signer,
            );

            token::transfer(cpi_ctx_seller, seller_payment)?;
        }

        // 3. Transfer refund to buyer
        if total_buyer_refund > 0 {
            let buyer_instruction = Transfer {
                from: ctx.accounts.escrow_token_account.to_account_info(),
                to: ctx.accounts.buyer_token_account.to_account_info(),
                authority: ctx.accounts.market_authority.to_account_info(),
            };

            let cpi_ctx_buyer = CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                buyer_instruction,
                signer,
            );

            token::transfer(cpi_ctx_buyer, total_buyer_refund)?;
        }

        // Update order status
        order.status = OrderStatus::PartiallyCompleted;
        order.completion_percentage = completion_percentage;

        // Update seller stats
        seller_stats.lifetime_volume = seller_stats
            .lifetime_volume
            .checked_add(payment_to_seller)
            .ok_or(ErrorCode::MathOverflow)?;
        seller_stats.active_orders = seller_stats.active_orders.saturating_sub(1);
        seller_stats.completed_orders += 1;

        Ok(())
    }

}

// Helper function to determine seller's fee percentage based on volume
fn get_seller_fee_percentage(market_state: &MarketState, lifetime_volume: u64) -> u16 {
    if lifetime_volume >= market_state.tier3_volume {
        market_state.tier3_fee
    } else if lifetime_volume >= market_state.tier2_volume {
        market_state.tier2_fee
    } else if lifetime_volume >= market_state.tier1_volume {
        market_state.tier1_fee
    } else {
        market_state.fee_percentage
    }
}

// Market state account
#[account]
pub struct MarketState {
    pub authority: Pubkey,      // Admin authority
    pub order_counter: u64,     // Counter for assigning unique order IDs
    pub deposit_percentage: u8, // Seller deposit percentage requirement
    pub fee_percentage: u16,    // Platform fee in basis points (e.g., 420 = 4.2%)
    pub fee_recipient: Pubkey,  // Account that receives platform fees

    // Tiered fee structure based on seller volume
    pub tier1_volume: u64, // Volume threshold for tier 1
    pub tier1_fee: u16,    // Fee percentage for tier 1 in basis points
    pub tier2_volume: u64, // Volume threshold for tier 2
    pub tier2_fee: u16,    // Fee percentage for tier 2 in basis points
    pub tier3_volume: u64, // Volume threshold for tier 3
    pub tier3_fee: u16,    // Fee percentage for tier 3 in basis points

    pub min_order_value: u64, // Minimum order value in USDC
}

// Order account
#[account]
pub struct Order {
    pub id: u64,                   // Unique order ID
    pub order_type: OrderType,     // Buy or Sell order
    pub buyer: Pubkey,             // Buyer's wallet address
    pub seller: Option<Pubkey>,    // Seller's wallet address (None until accepted)
    pub xnm_amount: u64,           // Amount of XNM tokens
    pub min_xnm_amount: u64,       // Min XNM amount for sell orders
    pub max_xnm_amount: u64,       // Max XNM amount for sell orders
    pub price: u64,                // Price per XNM token in USDC (6 decimals)
    pub created_at: i64,           // Unix timestamp when order was created
    pub deadline: i64,             // Deadline for order completion
    pub completion_days: u8,       // Days to complete for sell orders
    pub eth_address: String,       // Ethereum address for XNM delivery
    pub status: OrderStatus,       // Current status of the order
    pub total_value: u64,          // Total order value in USDC
    pub deposit_amount: u64,       // Seller's deposit amount
    pub completion_percentage: u8, // 0-100 percentage of order completion
}

// Seller statistics account
#[account]
pub struct SellerStats {
    pub seller: Pubkey,        // Seller's wallet address
    pub lifetime_volume: u64,  // Total lifetime volume in USDC
    pub active_orders: u32,    // Number of currently active orders
    pub completed_orders: u32, // Number of successfully completed orders
    pub is_initialized: bool,  // Flag to check if stats are initialized
}

// Order type enum
#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Eq)]
pub enum OrderType {
    Buy,
    Sell,
}

// Order status enum
#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Eq)]
pub enum OrderStatus {
    Open,
    InProgress,
    Completed,
    PartiallyCompleted,
    Cancelled,
    Refunded,
    Disputed,
}

// Context for initializing the program
#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(init, payer = authority, space = 8 + size_of::<MarketState>(), seeds = [b"market_state"], bump)]
    pub market_state: Account<'info, MarketState>,

    #[account(mut)]
    pub authority: Signer<'info>,

    pub usdc_mint: Account<'info, Mint>,

    #[account(
        init,
        payer = authority,
        associated_token::mint = usdc_mint,
        associated_token::authority = market_state,
    )]
    pub escrow_token_account: Account<'info, TokenAccount>,

    pub system_program: Program<'info, System>,
}

// Context for updating platform parameters
#[derive(Accounts)]
pub struct UpdateParams<'info> {
    #[account(mut)]
    pub market_state: Account<'info, MarketState>,

    pub authority: Signer<'info>,
}

// Context for creating a buy order
#[derive(Accounts)]
#[instruction(xnm_amount: u64, price: u64, deadline_days: u8, eth_address: String)]
pub struct CreateBuyOrder<'info> {
    #[account(mut)]
    pub market_state: Account<'info, MarketState>,

    #[account(
        init,
        payer = buyer,
        space = 8 + size_of::<Order>() + eth_address.len(),
        seeds = [b"order", market_state.order_counter.to_le_bytes().as_ref()],
        bump
    )]
    pub order: Account<'info, Order>,

    #[account(mut)]
    pub buyer: Signer<'info>,

    #[account(mut)]
    pub buyer_token_account: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = escrow_token_account.mint == buyer_token_account.mint,
        constraint = escrow_token_account.owner == market_state.key()
    )]
    pub escrow_token_account: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

// Context for creating a sell order
#[derive(Accounts)]
#[instruction(min_xnm_amount: u64, max_xnm_amount: u64, price: u64, days_to_complete: u8)]
pub struct CreateSellOrder<'info> {
    #[account(mut)]
    pub market_state: Account<'info, MarketState>,

    #[account(
        init,
        payer = seller,
        space = 8 + size_of::<Order>() + 42, // 42 is max length of ETH address
        seeds = [b"order", market_state.order_counter.to_le_bytes().as_ref()],
        bump
    )]
    pub order: Account<'info, Order>,

    #[account(mut)]
    pub seller: Signer<'info>,

    #[account(mut)]
    pub seller_token_account: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = escrow_token_account.mint == seller_token_account.mint,
        constraint = escrow_token_account.owner == market_state.key()
    )]
    pub escrow_token_account: Account<'info, TokenAccount>,

    #[account(
        init_if_needed,
        payer = seller,
        space = 8 + size_of::<SellerStats>(),
        seeds = [b"seller_stats", seller.key().as_ref()],
        bump
    )]
    pub seller_stats: Account<'info, SellerStats>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

// Context for taking a sell order
#[derive(Accounts)]
#[instruction(order_id: u64, xnm_amount: u64, eth_address: String)]
pub struct TakeSellOrder<'info> {
    #[account(seeds = [b"market_state"], bump)]
    pub market_state: Account<'info, MarketState>,

    #[account(
        mut,
        seeds = [b"order", order_id.to_le_bytes().as_ref()],
        bump,
        constraint = order.id == order_id
    )]
    pub order: Account<'info, Order>,

    #[account(mut)]
    pub buyer: Signer<'info>,

    #[account(mut)]
    pub buyer_token_account: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = escrow_token_account.mint == buyer_token_account.mint,
        constraint = escrow_token_account.owner == market_state.key()
    )]
    pub escrow_token_account: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = seller_token_account.owner == order.seller.unwrap()
    )]
    pub seller_token_account: Account<'info, TokenAccount>,

    #[account(
        mut,
        seeds = [b"seller_stats", order.seller.unwrap().as_ref()],
        bump
    )]
    pub seller_stats: Account<'info, SellerStats>,

    /// CHECK: This is safe because we only use it as a PDA signer
    #[account(seeds = [market_state.key().as_ref()], bump)]
    pub market_authority: AccountInfo<'info>,

    pub token_program: Program<'info, Token>,
}

// Context for accepting a buy order
#[derive(Accounts)]
#[instruction(order_id: u64)]
pub struct AcceptBuyOrder<'info> {
    #[account(seeds = [b"market_state"], bump)]
    pub market_state: Account<'info, MarketState>,

    #[account(
        mut,
        seeds = [b"order", order_id.to_le_bytes().as_ref()],
        bump,
        constraint = order.id == order_id
    )]
    pub order: Account<'info, Order>,

    #[account(mut)]
    pub seller: Signer<'info>,

    #[account(mut)]
    pub seller_token_account: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = escrow_token_account.mint == seller_token_account.mint,
        constraint = escrow_token_account.owner == market_state.key()
    )]
    pub escrow_token_account: Account<'info, TokenAccount>,

    #[account(
        init_if_needed,
        payer = seller,
        space = 8 + size_of::<SellerStats>(),
        seeds = [b"seller_stats", seller.key().as_ref()],
        bump
    )]
    pub seller_stats: Account<'info, SellerStats>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

// Context for completing an order
#[derive(Accounts)]
#[instruction(order_id: u64)]
pub struct CompleteOrder<'info> {
    #[account(seeds = [b"market_state"], bump)]
    pub market_state: Account<'info, MarketState>,

    #[account(
        mut,
        seeds = [b"order", order_id.to_le_bytes().as_ref()],
        bump,
        constraint = order.id == order_id
    )]
    pub order: Account<'info, Order>,

    #[account(mut, constraint = buyer.key() == order.buyer)]
    pub buyer: Signer<'info>,

    #[account(
        mut,
        constraint = escrow_token_account.owner == market_state.key()
    )]
    pub escrow_token_account: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = seller_token_account.owner == order.seller.unwrap()
    )]
    pub seller_token_account: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = fee_recipient_token_account.owner == market_state.fee_recipient
    )]
    pub fee_recipient_token_account: Account<'info, TokenAccount>,

    #[account(
        mut,
        seeds = [b"seller_stats", order.seller.unwrap().as_ref()],
        bump
    )]
    pub seller_stats: Account<'info, SellerStats>,

    /// CHECK: This is safe because we only use it as a PDA signer
    #[account(seeds = [market_state.key().as_ref()], bump)]
    pub market_authority: AccountInfo<'info>,

    pub token_program: Program<'info, Token>,
}

// Context for cancelling an order
#[derive(Accounts)]
#[instruction(order_id: u64)]
pub struct CancelOrder<'info> {
    #[account(seeds = [b"market_state"], bump)]
    pub market_state: Account<'info, MarketState>,

    #[account(
        mut,
        seeds = [b"order", order_id.to_le_bytes().as_ref()],
        bump,
        constraint = order.id == order_id
    )]
    pub order: Account<'info, Order>,

    #[account(mut)]
    pub caller: Signer<'info>,

    #[account(
        mut,
        constraint = escrow_token_account.owner == market_state.key()
    )]
    pub escrow_token_account: Account<'info, TokenAccount>,

    #[account(mut)]
    pub user_token_account: Account<'info, TokenAccount>,

    /// CHECK: This is safe because we only use it as a PDA signer
    #[account(seeds = [market_state.key().as_ref()], bump)]
    pub market_authority: AccountInfo<'info>,

    pub token_program: Program<'info, Token>,
}

// Context for resolving a dispute
#[derive(Accounts)]
#[instruction(order_id: u64, completion_percentage: u8, platform_fee_percentage: u8)]
pub struct ResolveDispute<'info> {
    #[account(seeds = [b"market_state"], bump)]
    pub market_state: Account<'info, MarketState>,

    #[account(
        mut,
        seeds = [b"order", order_id.to_le_bytes().as_ref()],
        bump,
        constraint = order.id == order_id
    )]
    pub order: Account<'info, Order>,

    #[account(mut, constraint = authority.key() == market_state.authority)]
    pub authority: Signer<'info>,

    #[account(
        mut,
        constraint = escrow_token_account.owner == market_state.key()
    )]
    pub escrow_token_account: Account<'info, TokenAccount>,

    #[account(mut)]
    pub buyer_token_account: Account<'info, TokenAccount>,

    #[account(mut)]
    pub seller_token_account: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = fee_recipient_token_account.owner == market_state.fee_recipient
    )]
    pub fee_recipient_token_account: Account<'info, TokenAccount>,

    #[account(
        mut,
        seeds = [b"seller_stats", order.seller.unwrap().as_ref()],
        bump
    )]
    pub seller_stats: Account<'info, SellerStats>,

    /// CHECK: This is safe because we only use it as a PDA signer
    #[account(seeds = [market_state.key().as_ref()], bump)]
    pub market_authority: AccountInfo<'info>,

    pub token_program: Program<'info, Token>,
}

// Error codes
#[error_code]
pub enum ErrorCode {
    #[msg("Unauthorized access")]
    Unauthorized,

    #[msg("Invalid order ID")]
    InvalidOrderId,

    #[msg("Invalid amount")]
    InvalidAmount,

    #[msg("Invalid price")]
    InvalidPrice,

    #[msg("Invalid deadline")]
    InvalidDeadline,

    #[msg("Invalid ETH address")]
    InvalidEthAddress,

    #[msg("Math overflow")]
    MathOverflow,

    #[msg("Order not open")]
    OrderNotOpen,

    #[msg("Order not in progress")]
    InvalidOrderStatus,

    #[msg("Order not cancellable")]
    OrderNotCancellable,

    #[msg("Wrong order type")]
    WrongOrderType,

    #[msg("Invalid percentage")]
    InvalidPercentage,

    #[msg("Order too small")]
    OrderTooSmall,
}

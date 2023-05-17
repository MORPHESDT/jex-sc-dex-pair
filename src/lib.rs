#![no_std]

use core::ops::Deref;

multiversx_sc::imports!();

mod fees;
mod liquidity;
mod swap;
mod wrap_sc_proxy;

#[multiversx_sc::contract]
pub trait JexScPairContract:
    fees::FeesModule + liquidity::LiquidityModule + swap::SwapModule
{
    #[init]
    fn init(&self, first_token: TokenIdentifier, second_token: TokenIdentifier) {
        self.first_token().set_if_empty(&first_token);
        self.second_token().set_if_empty(&second_token);
    }

    // owner endpoints

    #[only_owner]
    #[payable("EGLD")]
    #[endpoint(issueLpToken)]
    fn issue_lp_token(&self, lp_token_display_name: ManagedBuffer, lp_token_ticker: ManagedBuffer) {
        require!(self.lp_token().is_empty(), "LP token already issued");

        let egld_value = self.call_value().egld_value().deref().clone();
        let caller = self.blockchain().get_caller();

        self.send()
            .esdt_system_sc_proxy()
            .issue_fungible(
                egld_value,
                &lp_token_display_name,
                &lp_token_ticker,
                &BigUint::from(1_000u32),
                FungibleTokenProperties {
                    num_decimals: 18,
                    can_freeze: true,
                    can_wipe: true,
                    can_pause: true,
                    can_mint: true,
                    can_burn: true,
                    can_change_owner: true,
                    can_upgrade: true,
                    can_add_special_roles: true,
                },
            )
            .async_call()
            .with_callback(self.callbacks().lp_token_issue_callback(&caller))
            .call_and_exit();
    }

    #[only_owner]
    #[payable("*")]
    #[endpoint(addInitialLiquidity)]
    fn add_initial_liquidity(&self) {
        require!(!self.lp_token().is_empty(), "LP token not issued");

        let [first_payment, second_payment] = self.call_value().multi_esdt();

        require!(
            first_payment.token_identifier == self.first_token().get() && first_payment.amount > 0,
            "Invalid payment for first token"
        );

        require!(
            second_payment.token_identifier == self.second_token().get()
                && second_payment.amount > 0,
            "Invalid payment for second token"
        );

        let (lp_amount, lp_token) =
            self.lp_add_initial_liquidity(&first_payment.amount, &second_payment.amount);

        let caller = self.blockchain().get_caller();
        self.send().direct_esdt(&caller, &lp_token, 0, &lp_amount);
    }

    /// Configure liquidity providers swap fees
    /// 100 = 1%
    #[only_owner]
    #[endpoint(configureLiqProvidersFees)]
    fn configure_liq_providers_fees(&self, fees: u32) {
        self.liq_providers_fees().set(fees);
    }

    /// Configure platform swap fees
    /// 100 = 1%
    #[only_owner]
    #[endpoint(configurePlatformFees)]
    fn configure_platform_fees(&self, fees: u32, receiver: ManagedAddress) {
        self.platform_fees().set(fees);
        self.platform_fees_receiver().set(&receiver);
    }

    // public endpoints

    #[payable("*")]
    #[endpoint(addLiquidity)]
    fn add_liquidity(&self, min_second_token_amount: BigUint) {
        let [first_payment, second_payment] = self.call_value().multi_esdt();

        require!(
            first_payment.token_identifier == self.first_token().get() && first_payment.amount > 0,
            "Invalid payment for first token"
        );

        require!(
            second_payment.token_identifier == self.second_token().get()
                && second_payment.amount > 0,
            "Invalid payment for second token"
        );

        let (lp_amount, lp_token, overpaid_second_token_amount) = self.lp_add_liquidity(
            &first_payment.amount,
            &min_second_token_amount,
            &second_payment.amount,
        );

        let caller = self.blockchain().get_caller();
        self.send().direct_esdt(&caller, &lp_token, 0, &lp_amount);

        if overpaid_second_token_amount > 0 {
            self.send().direct_esdt(
                &caller,
                &second_payment.token_identifier,
                second_payment.token_nonce,
                &overpaid_second_token_amount,
            );
        }
    }

    /// Add liquidity by providing only 1 of the 2 tokens
    /// Provided liquidity is added to the reserves and corresponding LP tokens are sent to caller.
    /// payment = token to deposit
    #[payable("*")]
    #[endpoint(addLiquiditySingle)]
    fn add_liquidity_single(
        &self,
        min_first_token_amount: BigUint,
        min_second_token_amount: BigUint,
    ) {
        let (token_identifier, payment_amount) = self.call_value().single_fungible_esdt();

        let first_token = self.first_token().get();
        let second_token = self.second_token().get();

        let is_first_token_in = token_identifier == first_token;
        let is_second_token_in = token_identifier == second_token;

        require!(
            is_first_token_in || is_second_token_in,
            "Invalid payment token"
        );

        let (lp_amount, lp_token) = self.lp_add_liquidity_single_side(
            &payment_amount,
            &min_first_token_amount,
            &min_second_token_amount,
            is_first_token_in,
        );

        let caller = self.blockchain().get_caller();
        self.send().direct_esdt(&caller, &lp_token, 0, &lp_amount);
    }

    #[payable("*")]
    #[endpoint(removeLiquidity)]
    fn remove_liquidity(&self, min_first_token_amount: BigUint, min_second_token_amount: BigUint) {
        let (lp_token, lp_amount) = self.call_value().single_fungible_esdt();

        let (exact_first_token_amount, exact_second_token_amount) =
            self.lp_remove_liquidity(lp_token, lp_amount);

        require!(
            exact_first_token_amount >= min_first_token_amount,
            "Max slippage exceeded for first token"
        );
        require!(
            exact_second_token_amount >= min_second_token_amount,
            "Max slippage exceeded for second token"
        );

        let caller = self.blockchain().get_caller();
        self.send().direct_esdt(
            &caller,
            &self.first_token().get(),
            0,
            &exact_first_token_amount,
        );
        self.send().direct_esdt(
            &caller,
            &self.second_token().get(),
            0,
            &exact_second_token_amount,
        );
    }

    /// Remove liquidity and swap one half to desired token in 1 transaction
    #[payable("*")]
    #[endpoint(removeLiquiditySingle)]
    fn remove_liquidity_single(
        &self,
        token_out: TokenIdentifier,
        min_first_token_amount: BigUint,
        min_second_token_amount: BigUint,
    ) {
        let (lp_token, lp_amount) = self.call_value().single_fungible_esdt();

        let (first_tokens_removed, second_tokens_removed) =
            self.lp_remove_liquidity(lp_token, lp_amount);

        let is_first_token_out = &token_out == &self.first_token().get();
        let is_first_token_in = !is_first_token_out;

        let swap_amount_in = if is_first_token_in {
            &first_tokens_removed
        } else {
            &second_tokens_removed
        };

        let swap_payment =
            self.swap_tokens_fixed_input_inner(swap_amount_in, &token_out, is_first_token_in);

        let caller = self.blockchain().get_caller();
        if is_first_token_out {
            let amount_out = &first_tokens_removed + &swap_payment.amount;
            require!(
                amount_out >= min_first_token_amount,
                "Max slippage exceeded for first token"
            );

            self.send()
                .direct_esdt(&caller, &self.first_token().get(), 0, &amount_out);
        } else {
            let amount_out = &second_tokens_removed + &swap_payment.amount;
            require!(
                amount_out >= min_second_token_amount,
                "Max slippage exceeded for second token"
            );
            self.send()
                .direct_esdt(&caller, &self.second_token().get(), 0, &amount_out);
        }
    }

    #[payable("*")]
    #[endpoint(swapTokensFixedInput)]
    fn swap_tokens_fixed_input(&self, min_amount_out: BigUint) {
        let (token_in, amount_in) = self.call_value().single_fungible_esdt();

        let first_token = self.first_token().get();
        let second_token = self.second_token().get();

        let is_first_token_in = token_in == first_token;
        let is_second_token_in = token_in == second_token;

        require!(
            is_first_token_in || is_second_token_in,
            "Invalid payment token"
        );

        let token_out = if is_first_token_in {
            second_token
        } else {
            first_token
        };

        let payment_out =
            self.swap_tokens_fixed_input_inner(&amount_in, &token_out, is_first_token_in);

        require!(
            payment_out.amount >= min_amount_out,
            "Max slippage exceeded"
        );

        let caller = self.blockchain().get_caller();
        self.send().direct_esdt(
            &caller,
            &payment_out.token_identifier,
            payment_out.token_nonce,
            &payment_out.amount,
        );
    }

    #[payable("*")]
    #[endpoint(swapTokensFixedOutput)]
    fn swap_tokens_fixed_output(&self, exact_amount_out: BigUint) {
        let (token_in, amount_in) = self.call_value().single_fungible_esdt();

        let first_token = self.first_token().get();
        let second_token = self.second_token().get();

        let is_first_token_in = token_in == first_token;
        let is_second_token_in = token_in == second_token;

        require!(
            is_first_token_in || is_second_token_in,
            "Invalid payment token"
        );

        let token_out = if is_first_token_in {
            second_token
        } else {
            first_token
        };

        let exact_amount_in =
            self.swap_tokens_fixed_output_inner(&exact_amount_out, &token_out, is_first_token_in);

        require!(exact_amount_in <= amount_in, "Max slippage exceeded");

        let caller = self.blockchain().get_caller();
        self.send()
            .direct_esdt(&caller, &token_out, 0, &exact_amount_out);

        if amount_in > exact_amount_in {
            self.send()
                .direct_esdt(&caller, &token_in, 0, &(amount_in - exact_amount_in));
        }
    }

    // storage & views

    #[view(estimateAmountIn)]
    fn estimate_amount_in(
        &self,
        token_out: TokenIdentifier,
        amount_out: BigUint,
    ) -> swap::EstimateAmountIn<Self::Api> {
        let first_token = self.first_token().get();
        let second_token = self.second_token().get();

        let is_first_token_out = token_out == first_token;
        let is_second_token_out = token_out == second_token;

        require!(
            is_first_token_out || is_second_token_out,
            "Invalid payment token"
        );

        let is_first_token_in = !is_first_token_out;

        let estimation = self.estimate_amount_in_inner(&amount_out, is_first_token_in);

        estimation
    }

    #[view(estimateAmountOut)]
    fn estimate_amount_out(
        &self,
        token_in: TokenIdentifier,
        amount_in: BigUint,
    ) -> swap::EstimateAmountOut<Self::Api> {
        let first_token = self.first_token().get();
        let second_token = self.second_token().get();

        let is_first_token_in = token_in == first_token;
        let is_second_token_in = token_in == second_token;

        require!(
            is_first_token_in || is_second_token_in,
            "Invalid payment token"
        );

        let estimation = self.estimate_amount_out_inner(&amount_in, is_first_token_in);

        estimation
    }

    #[view(estimateAddLiquiditySingle)]
    fn estimate_add_liquidity_single(
        &self,
        token_in: TokenIdentifier,
        amount_in: BigUint,
    ) -> liquidity::EstimateAddLiquidityOut<Self::Api> {
        let first_token = self.first_token().get();
        let second_token = self.second_token().get();

        let is_first_token_in = token_in == first_token;
        let is_second_token_in = token_in == second_token;

        require!(
            is_first_token_in || is_second_token_in,
            "Invalid payment token"
        );

        let estimation = self.lp_estimate_add_liquidity_single(&amount_in, is_first_token_in);

        estimation
    }

    #[view(estimateRemoveLiquidity)]
    fn estimate_remove_liquidity(
        &self,
        lp_amount: BigUint,
    ) -> liquidity::EstimateRemoveLiquidityOut<Self::Api> {
        let estimation = self.lp_estimate_remove_liquidity(&lp_amount);

        estimation
    }

    /// Estimate liquidity removal to one token
    /// (liquidity removal + swap of one half to desired token)
    #[view(estimateRemoveLiquiditySingle)]
    fn estimate_remove_liquidity_single(
        &self,
        lp_amount: BigUint,
        token_out: TokenIdentifier,
    ) -> liquidity::EstimateRemoveLiquidityOut<Self::Api> {
        require!(
            &lp_amount * 2u32 <= self.lp_token_supply().get(),
            "Cannot remove that much liquidity"
        );

        let est_remove_lp = self.lp_estimate_remove_liquidity(&lp_amount);

        let first_token = self.first_token().get();
        let second_token = self.second_token().get();

        let is_first_token_out = token_out == first_token;
        let is_second_token_out = token_out == second_token;

        require!(
            is_first_token_out || is_second_token_out,
            "Invalid out token"
        );

        self.first_token_reserve()
            .update(|x| *x -= &est_remove_lp.eq_first_tokens);
        self.second_token_reserve()
            .update(|x| *x -= &est_remove_lp.eq_second_tokens);

        let half_swap_estimate = if is_first_token_out {
            self.estimate_amount_out_inner(&est_remove_lp.eq_second_tokens, false)
        } else {
            self.estimate_amount_out_inner(&est_remove_lp.eq_first_tokens, true)
        };

        let estimation = liquidity::EstimateRemoveLiquidityOut {
            eq_first_tokens: if is_first_token_out {
                &est_remove_lp.eq_first_tokens + &half_swap_estimate.net_amount_out
            } else {
                BigUint::zero()
            },
            eq_second_tokens: if is_second_token_out {
                &est_remove_lp.eq_second_tokens + &half_swap_estimate.net_amount_out
            } else {
                BigUint::zero()
            },
        };

        estimation
    }

    #[view(getFirstToken)]
    #[storage_mapper("first_token")]
    fn first_token(&self) -> SingleValueMapper<TokenIdentifier>;

    #[view(getSecondToken)]
    #[storage_mapper("second_token")]
    fn second_token(&self) -> SingleValueMapper<TokenIdentifier>;

    #[view(getWrapScAddress)]
    #[storage_mapper("wrap_sc_address")]
    fn wrap_sc_address(&self) -> SingleValueMapper<ManagedAddress>;

    // callbacks

    #[callback]
    fn lp_token_issue_callback(
        &self,
        caller: &ManagedAddress,
        #[call_result] result: ManagedAsyncCallResult<()>,
    ) {
        let (token_id, returned_tokens) = self.call_value().egld_or_single_fungible_esdt();
        match result {
            ManagedAsyncCallResult::Ok(()) => {
                self.lp_token().set(token_id.unwrap_esdt());
            }
            ManagedAsyncCallResult::Err(_) => {
                if token_id.is_egld() && returned_tokens > 0u64 {
                    self.send().direct_egld(caller, &returned_tokens);
                }
            }
        }
    }

    // proxies

    #[proxy]
    fn wrap_sc_proxy(&self, sc_address: ManagedAddress) -> wrap_sc_proxy::Proxy<Self::Api>;
}

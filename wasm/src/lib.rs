// Code generated by the multiversx-sc multi-contract system. DO NOT EDIT.

////////////////////////////////////////////////////
////////////////// AUTO-GENERATED //////////////////
////////////////////////////////////////////////////

// Init:                                 1
// Endpoints:                           19
// Async Callback (empty):               1
// Total number of exported functions:  21

#![no_std]
#![feature(lang_items)]

multiversx_sc_wasm_adapter::allocator!();
multiversx_sc_wasm_adapter::panic_handler!();

multiversx_sc_wasm_adapter::endpoints! {
    jex_sc_pair
    (
        addInitialLiquidity
        configureLiqProvidersFees
        configurePlatformFees
        addLiquidity
        removeLiquidity
        swapTokensFixedInput
        swapTokensFixedOutput
        estimateAmountIn
        estimateAmountOut
        getFirstToken
        getSecondToken
        getWrapScAddress
        getLiqProvidersFees
        getPlatformFees
        getPlatformFeesReceiver
        getFirstTokenReserve
        getSecondTokenReserve
        getLpToken
        getLpTokenSupply
    )
}

multiversx_sc_wasm_adapter::empty_callback! {}

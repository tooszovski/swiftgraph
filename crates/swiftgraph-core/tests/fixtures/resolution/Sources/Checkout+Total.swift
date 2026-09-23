extension Checkout {
    func total() {
        pricing.compute()
        self.tax.compute()
        PricingService.shared.compute()
    }
}

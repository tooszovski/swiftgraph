final class PricingService {
    static let shared = PricingService()
    func compute() {}
}

final class TaxService {
    func compute() {}
}

final class Checkout {
    let pricing: PricingService
    var tax = TaxService()

    init(pricing: PricingService) {
        self.pricing = pricing
    }
}

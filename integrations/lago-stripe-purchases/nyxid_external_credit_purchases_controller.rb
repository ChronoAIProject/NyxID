# SPDX-License-Identifier: AGPL-3.0-only

module Api
  module V1
    class NyxidExternalCreditPurchasesController < Api::BaseController
      class IneligibleWallet < StandardError; end
      class InvalidPayment < StandardError; end

      class UnprovenRefund < StandardError; end
      class RefundUsageUnavailable < StandardError; end

      before_action :authorize_integration

      rescue_from IneligibleWallet, with: :reject_wallet
      rescue_from InvalidPayment, with: :reject_payment
      rescue_from UnprovenRefund, with: :reject_refund
      rescue_from RefundUsageUnavailable, with: :retry_refund
      rescue_from ActiveRecord::RecordNotUnique, with: :reject_identity

      def capability
        eligible_wallet
        # Check the reversal migration and route on the current instance.
        NyxidExternalCreditPurchase.limit(1).pluck(:id)
        NyxidExternalCreditRefund.limit(1).pluck(:decision_id)
        unless NyxidExternalCreditRefund.columns_hash.fetch("wallet_transaction_id").null &&
            Rails.application.routes.url_helpers.respond_to?(:api_v1_cancel_nyxid_external_credit_reversal_path) &&
            reversal_indexes_ready?
          return render json: {code: "external_purchase_reversal_capability_unavailable"}, status: :service_unavailable
        end
        render json: {version: 2, wallet_id: params[:wallet_id], invoice_created: false,
          stripe_account_id: ENV.fetch("NYXID_CREDIT_PURCHASE_STRIPE_ACCOUNT_ID"), livemode: stripe_livemode?}
      end

      def show
        receipt = purchases.find_by(purchase_id: params[:purchase_id], wallet_id: params[:wallet_id])
        return not_found_error(resource: "external_credit_purchase") unless receipt

        render_receipt(receipt)
      end

      def create
        attributes = payment_attributes
        receipt = Wallet.transaction do
          wallet = eligible_wallet(lock: true)
          existing = purchases.find_by(purchase_id: attributes.fetch(:purchase_id))
          if existing
            raise InvalidPayment unless existing.wallet_id == wallet.id &&
              attributes.all? { |key, value| existing.public_send(key) == value }
            next existing
          end

          wallet_credit = WalletCredit.new(wallet:, credit_amount: BigDecimal(attributes.fetch(:credits)))
          lot = ::WalletTransactions::CreateService.call!(
            wallet:,
            wallet_credit:,
            transaction_type: :inbound,
            status: :pending,
            transaction_status: :purchased,
            source: :manual,
            priority: 50,
            metadata: [{key: "nyxid_purchase_id", value: attributes.fetch(:purchase_id)}]
          ).wallet_transaction

          # The immutable external payment receipt and purchased balance commit
          # together. Stripe owns the customer invoice and collects the money.
          saved = purchases.create!(**attributes, wallet:, wallet_transaction: lot)
          ::Wallets::ApplyPaidCreditsService.call!(wallet_transaction: lot)
          saved
        end
        render_receipt(receipt)
      end

      def refund
        reverse_credits(cancel_only: false)
      end

      def cancel_refund
        reverse_credits(cancel_only: true)
      end

      private

      def reversal_indexes_ready?
        NyxidExternalCreditRefund.with_connection do |connection|
          connection.select_value(<<~SQL)
            SELECT to_regclass('index_nyxid_external_credit_refunds_on_stripe_refund_id') IS NULL
              AND 2 = (
                SELECT count(*)
                FROM pg_index
                WHERE indexrelid IN (
                  to_regclass('index_nyxid_external_credit_refunds_on_decision_id'),
                  to_regclass('idx_nyxid_external_applied_reversals')
                )
                AND indisvalid AND indisunique
              )
          SQL
        end
      end

      def reverse_credits(cancel_only:)
        raw = params.require(:refund)
        refund_id = raw[:stripe_refund_id]
        decision_id = raw[:decision_id]
        credits = raw[:credits]
        amount_minor = raw[:amount_minor]
        protected_minor = raw[:protected_minor]
        raise InvalidPayment unless refund_id.is_a?(String) && refund_id.match?(/\A(?:re|dp)_[a-zA-Z0-9_]{1,124}\z/) &&
          decision_id.is_a?(String) && decision_id.match?(/\A[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}\z/) &&
          credits.is_a?(Integer) && credits.between?(1, 10_000_000) &&
          amount_minor.is_a?(Integer) && amount_minor.between?(1, 99_999_999) &&
          protected_minor.is_a?(Integer) && protected_minor.between?(0, 999_999_999_999)

        customer = eligible_wallet.customer
        record = Wallet.transaction do
          ::Customers::LockService.call(customer:, scope: :prepaid_credit) do
            wallet = eligible_wallet(lock: true)
            purchase = purchases.find_by!(purchase_id: params[:purchase_id], wallet_id: wallet.id)
            refunds = NyxidExternalCreditRefund.where(purchase:)
            if (prior = refunds.find_by(decision_id:))
              raise InvalidPayment unless prior.stripe_refund_id == refund_id &&
                prior.credits == credits && prior.amount_minor == amount_minor
              next prior
            end
            if cancel_only
              next refunds.create!(stripe_refund_id: refund_id, decision_id:, credits:, amount_minor:)
            end
            applied = refunds.where.not(wallet_transaction_id: nil)
            raise InvalidPayment if applied.exists?(stripe_refund_id: refund_id)

            protected_minor = [protected_minor, provider_accrued_minor(customer)].max
            lot = purchase.wallet_transaction
            wallet_credit = WalletCredit.new(wallet:, credit_amount: BigDecimal(credits))
            refundable = lot.settled? && lot.purchased? && lot.inbound? &&
              lot.settled_at > 365.days.ago &&
              applied.sum(:credits) + credits <= purchase.credits &&
              wallet.balance_cents - protected_minor >= wallet_credit.amount_cents
            if refundable && refund_id.start_with?("re_")
              refunded = applied.where("stripe_refund_id LIKE ?", "re\_%").sum(:amount_minor)
              refundable = refunded + amount_minor <= purchase.total_minor
            end
            if refundable
              begin
                refundable = unused_credits(wallet, lot, protected_minor) >= wallet_credit.amount_cents
              rescue UnprovenRefund
                refundable = false
              end
            end
            unless refundable
              next refunds.create!(stripe_refund_id: refund_id, decision_id:, credits:, amount_minor:)
            end

            void = ::WalletTransactions::VoidService.call!(
              wallet:, wallet_credit:, inbound_wallet_transaction: lot,
              metadata: [{key: "stripe_refund_id", value: refund_id},
                {key: "nyxid_reversed_purchase_transaction_id", value: lot.id}]
            ).wallet_transaction
            refunds.create!(wallet_transaction: void, stripe_refund_id: refund_id, decision_id:, credits:, amount_minor:)
          end
        end
        return reject_refund unless record.wallet_transaction_id
        render json: {refund: record.as_json(only: %i[stripe_refund_id decision_id credits amount_minor wallet_transaction_id])}
      end

      def provider_accrued_minor(customer)
        incomplete = customer.invoices.where(invoice_type: :subscription).where.not(status: %i[finalized voided])
        raise RefundUsageUnavailable if incomplete.exists?

        customer.active_subscriptions.sum do |subscription|
          usage = ::Invoices::CustomerUsageService.call(
            customer:, subscription:, apply_taxes: true, with_cache: false
          )
          raise RefundUsageUnavailable unless usage.success?

          usage.usage.total_amount_cents
        end
      end

      def unused_credits(wallet, lot, protected_minor)
        remainders = if wallet.traceable?
          wallet.wallet_transactions.available_inbound.in_consumption_order.limit(10_001)
            .pluck(:id, :remaining_amount_cents).to_h
        else
          nontraceable_remainders(wallet)
        end
        raise UnprovenRefund if remainders.length > 10_000
        remainders.each_pair do |id, remaining|
          protected = [protected_minor, remaining].min
          protected_minor -= protected
          return remaining - protected if id == lot.id
        end
        0
      end

      def nontraceable_remainders(wallet)
        # Dedicated wallets admit only external purchased lots. Stock usage
        # and expiry consume the oldest surviving credits. A linked reversal
        # reduces the particular lot its operator approved.
        history = wallet.wallet_transactions.settled.order(:settled_at, :id).limit(10_001).to_a
        raise UnprovenRefund if history.length > 10_000
        recorded = purchases.where(wallet:).pluck(:wallet_transaction_id).to_set
        remainders = {}
        history.each do |transaction|
          amount = transaction.amount_cents
          raise UnprovenRefund if amount.negative?
          if transaction.inbound?
            raise UnprovenRefund unless transaction.purchased? && recorded.include?(transaction.id)
            remainders[transaction.id] = amount
            next
          end
          target = transaction.metadata.find { |item| item["key"] == "nyxid_reversed_purchase_transaction_id" }&.fetch("value")
          if target
            raise UnprovenRefund unless remainders.fetch(target, 0) >= amount
            remainders[target] -= amount
          else
            remainders.each_key do |id|
              consumed = [remainders[id], amount].min
              remainders[id] -= consumed
              amount -= consumed
              break if amount.zero?
            end
            raise UnprovenRefund unless amount.zero?
          end
        end
        raise UnprovenRefund unless remainders.values.sum == wallet.balance_cents
        remainders
      end

      def authorize_integration
        expected_key = ENV["NYXID_CREDIT_PURCHASE_API_KEY_ID"]
        expected_secret = ENV["NYXID_CREDIT_PURCHASE_SECRET"]
        account = ENV["NYXID_CREDIT_PURCHASE_STRIPE_ACCOUNT_ID"]
        provided = request.headers["X-NyxID-Credit-Purchase-Key"].to_s
        return if expected_key.present? && expected_key == current_api_key.id.to_s &&
          expected_secret.to_s.bytesize >= 32 && provided.bytesize <= 512 && account.to_s.start_with?("acct_") &&
          ActiveSupport::SecurityUtils.secure_compare(Digest::SHA256.digest(expected_secret), Digest::SHA256.digest(provided))

        render json: {code: "external_purchase_integration_unauthorized"}, status: :forbidden
      end

      def eligible_wallet(lock: false)
        wallets = Wallet.where(organization_id: current_organization.id)
        wallets = wallets.lock if lock
        wallet = wallets.find_by(id: params[:wallet_id])
        raise IneligibleWallet unless wallet&.active? && wallet.currency == "USD" && wallet.rate_amount == 1 &&
          !wallet.recurring_transaction_rules.exists?

        customer = lock ? Customer.lock.find(wallet.customer_id) : wallet.customer
        raise IneligibleWallet unless customer.external_id == params[:external_customer_id] &&
          customer.payment_provider.blank? && !customer.payment_provider_customers.exists?

        wallet
      end

      def stripe_livemode?
        ENV.fetch("NYXID_CREDIT_PURCHASE_STRIPE_MODE", "test") == "live"
      end

      def purchases
        NyxidExternalCreditPurchase.where(organization_id: current_organization.id)
      end

      def payment_attributes
        raw = params.require(:purchase)
        strings = {
          purchase_id: "btu_",
          stripe_account_id: "acct_",
          stripe_customer_id: "cus_",
          stripe_checkout_id: stripe_livemode? ? "cs_live_" : "cs_test_",
          stripe_payment_intent_id: "pi_"
        }.to_h do |field, prefix|
          value = raw[field]
          raise InvalidPayment unless value.is_a?(String) && value.size <= 128 &&
            value.match?(/\A[a-zA-Z0-9_]+\z/) && value.start_with?(prefix)
          [field, value]
        end
        amounts = %i[credits subtotal_minor tax_minor total_minor].to_h do |field|
          value = raw[field]
          raise InvalidPayment unless value.is_a?(Integer) && value.between?(0, 999_999_999_999)
          [field, value]
        end
        raise InvalidPayment unless strings[:stripe_account_id] == ENV.fetch("NYXID_CREDIT_PURCHASE_STRIPE_ACCOUNT_ID") &&
          raw[:currency] == "usd" && amounts[:credits].between?(1, 10_000_000) &&
          amounts[:subtotal_minor] >= 1_000 && amounts[:subtotal_minor] + amounts[:tax_minor] == amounts[:total_minor]

        strings.merge(amounts).merge(currency: "usd")
      end

      def render_receipt(receipt)
        lot = receipt.wallet_transaction.reload
        render json: {purchase: receipt.as_json(only: %i[
          purchase_id wallet_id wallet_transaction_id stripe_account_id stripe_customer_id
          stripe_checkout_id stripe_payment_intent_id currency credits subtotal_minor tax_minor total_minor
        ]).merge(
          status: lot.status,
          transaction_type: lot.transaction_type,
          transaction_status: lot.transaction_status,
          settled_at: lot.settled_at
        )}
      end

      def reject_wallet
        render json: {code: "external_purchase_wallet_ineligible"}, status: :unprocessable_entity
      end

      def reject_payment
        render json: {code: "external_purchase_payment_mismatch"}, status: :unprocessable_entity
      end

      def reject_refund
        render json: {code: "external_purchase_refund_remainder_unproven"}, status: :unprocessable_entity
      end

      def retry_refund
        render json: {code: "external_purchase_refund_usage_unavailable"}, status: :service_unavailable
      end

      def reject_identity
        render json: {code: "external_purchase_payment_conflict"}, status: :conflict
      end

      def mode
        "write"
      end

      def resource_name
        "wallet_transaction"
      end
    end
  end
end

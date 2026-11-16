// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Shows a save to Photos while it runs, and then that it's done. The share sheet has closed by
// then, and a roll of DNGs takes a while to copy, so without this nothing says it's happening.
// It floats over the window without blocking it, and the rest of the app stays usable.

import UIKit

final class SaveHUD: UIView {
    private let total: Int
    private let name: String
    private let spinner = UIActivityIndicatorView(style: .large)
    private let bar = UIProgressView(progressViewStyle: .default)
    private let icon = UIImageView()
    private let title = UILabel()
    private let detail = UILabel()
    private let stopButton: UIButton
    private var done = false

    /// `name` is the DNG's file name when there's one. `stop` asks the save to stop after the
    /// DNG it's on, which only a save of several can.
    init(total: Int, name: String, stop: @escaping () -> Void) {
        self.total = total
        self.name = name
        var button: UIButton.Configuration
        if #available(iOS 26, *) { button = .glass() } else { button = .gray() }
        button.title = "Stop"
        stopButton = UIButton(configuration: button)
        super.init(frame: .zero)
        // Stopping waits for the DNG being saved.
        stopButton.addAction(UIAction { [weak self] _ in
            self?.stopButton.isEnabled = false
            stop()
        }, for: .primaryActionTriggered)

        let background: UIVisualEffectView
        if #available(iOS 26, *) {
            background = UIVisualEffectView(effect: UIGlassEffect(style: .regular))
            background.cornerConfiguration = .corners(radius: .fixed(28))
        } else {
            background = UIVisualEffectView(effect: UIBlurEffect(style: .systemMaterial))
            background.layer.cornerRadius = 20
            background.layer.cornerCurve = .continuous
            background.clipsToBounds = true
        }

        // The top holds the spinner while one DNG saves, and the mark once done. Several DNGs
        // show a bar under the title instead, with their count under it.
        let indicator = UIView()
        for view in [spinner, icon] as [UIView] {
            view.translatesAutoresizingMaskIntoConstraints = false
            indicator.addSubview(view)
            NSLayoutConstraint.activate([
                view.centerXAnchor.constraint(equalTo: indicator.centerXAnchor),
                view.centerYAnchor.constraint(equalTo: indicator.centerYAnchor),
            ])
        }
        bar.widthAnchor.constraint(equalToConstant: 168).isActive = true
        icon.preferredSymbolConfiguration = UIImage.SymbolConfiguration(pointSize: 40, weight: .regular)
        icon.tintColor = .label
        icon.isHidden = true
        spinner.startAnimating()
        indicator.isHidden = total > 1
        bar.isHidden = total == 1

        title.font = .preferredFont(forTextStyle: .headline)
        detail.font = .monospacedDigitSystemFont(
            ofSize: UIFont.preferredFont(forTextStyle: .subheadline).pointSize, weight: .regular)
        detail.textColor = .secondaryLabel
        for label in [title, detail] {
            label.textAlignment = .center
            label.numberOfLines = 0
            label.adjustsFontForContentSizeCategory = true
        }
        // A file name keeps its start and its end.
        detail.numberOfLines = 1
        detail.lineBreakMode = .byTruncatingMiddle
        stopButton.isHidden = total == 1

        let stack = UIStackView(arrangedSubviews: [indicator, title, bar, detail, stopButton])
        stack.axis = .vertical
        stack.alignment = .center
        stack.spacing = 6
        stack.setCustomSpacing(14, after: indicator)
        stack.setCustomSpacing(12, after: title)
        stack.setCustomSpacing(8, after: bar)
        stack.setCustomSpacing(14, after: detail)
        stack.translatesAutoresizingMaskIntoConstraints = false
        background.contentView.addSubview(stack)
        background.translatesAutoresizingMaskIntoConstraints = false
        addSubview(background)
        NSLayoutConstraint.activate([
            indicator.heightAnchor.constraint(equalToConstant: 48),
            indicator.widthAnchor.constraint(equalToConstant: 48),
            stack.topAnchor.constraint(equalTo: background.contentView.topAnchor, constant: 22),
            stack.bottomAnchor.constraint(equalTo: background.contentView.bottomAnchor, constant: -22),
            stack.leadingAnchor.constraint(equalTo: background.contentView.leadingAnchor, constant: 24),
            stack.trailingAnchor.constraint(equalTo: background.contentView.trailingAnchor, constant: -24),
            background.topAnchor.constraint(equalTo: topAnchor),
            background.bottomAnchor.constraint(equalTo: bottomAnchor),
            background.leadingAnchor.constraint(equalTo: leadingAnchor),
            background.trailingAnchor.constraint(equalTo: trailingAnchor),
            widthAnchor.constraint(greaterThanOrEqualToConstant: 220),
            widthAnchor.constraint(lessThanOrEqualToConstant: 280),
        ])

        title.text = "Saving to Photos"
        update(done: 0)
        addGestureRecognizer(UITapGestureRecognizer(target: self, action: #selector(tapped)))
        accessibilityViewIsModal = false
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) is not used") }

    /// Puts it in the middle of the window, over whatever is shown there.
    func show(in window: UIWindow) {
        translatesAutoresizingMaskIntoConstraints = false
        window.addSubview(self)
        NSLayoutConstraint.activate([
            centerXAnchor.constraint(equalTo: window.centerXAnchor),
            centerYAnchor.constraint(equalTo: window.centerYAnchor),
            widthAnchor.constraint(lessThanOrEqualTo: window.widthAnchor, constant: -32),
        ])
        alpha = 0
        if !UIAccessibility.isReduceMotionEnabled { transform = CGAffineTransform(scaleX: 0.94, y: 0.94) }
        UIView.animate(withDuration: 0.2, delay: 0, options: .curveEaseOut) {
            self.alpha = 1
            self.transform = .identity
        }
        UIAccessibility.post(notification: .announcement, argument: "Saving to Photos")
    }

    /// How many DNGs have been dealt with so far.
    func update(done: Int) {
        detail.text = total == 1 ? name : "\(done) of \(total)"
        bar.setProgress(Float(done) / Float(total), animated: done > 0)
    }

    /// Says what the save did, then fades after a moment. A tap puts it away sooner.
    func finish(saved: Int, stopped: Bool) {
        done = true
        spinner.stopAnimating()
        spinner.isHidden = true
        icon.superview?.isHidden = false
        bar.isHidden = true
        stopButton.isHidden = true
        icon.isHidden = false
        icon.image = UIImage(systemName: stopped ? "stop.circle" : "checkmark.circle")
        if !stopped { icon.addSymbolEffect(.bounce) }
        title.text = stopped ? "Stopped" : "Saved to Photos"
        detail.text = stopped ? "Saved \(saved) of \(total)" : total == 1 ? name : "\(total) DNGs"
        UIAccessibility.post(
            notification: .announcement,
            argument: stopped ? "Stopped. Saved \(saved) of \(total) to Photos" : "Saved to Photos")
        DispatchQueue.main.asyncAfter(deadline: .now() + 1.8) { [weak self] in self?.dismiss() }
    }

    /// Goes away at once, for a save that ends with an alert instead.
    func dismiss() {
        guard superview != nil else { return }
        isUserInteractionEnabled = false
        UIView.animate(withDuration: 0.25, delay: 0, options: .curveEaseIn) {
            self.alpha = 0
        } completion: { _ in
            self.removeFromSuperview()
        }
    }

    @objc private func tapped() {
        if done { dismiss() }
    }
}

import 'dart:async';
import 'dart:ui';

import 'package:flutter/material.dart';

import '../providers/connection_provider.dart';
import '../theme/app_theme.dart';

// ── Auth-screen color tokens (not in shared TelegramColors) ────────────────
const Color _kGradientStart = Color(0xFF3CA5FF);
const Color _kGradientEnd = Color(0xFF007AFF);
const Color _kInputBg = Color(0x990E1621);
const Color _kErrorText = Color(0xFFFC8181);
const Color _kErrorBg = Color(0x1AFF0000);
const Color _kErrorBorder = Color(0x33FF0000);
const Color _kAmberStart = Color(0xFFFFAE00);
const Color _kAmberEnd = Color(0xFFFF9500);
const Color _kBlurBlue = Color(0x33448AFF);
const Color _kBlurPurple = Color(0x1A8B5CF6);
const Color _kFloodRedBg = Color(0x33FF0000);
const Color _kFloodCountdown = Color(0xFF60A5FA);
const Color _kMutedGray = Color(0xFF9CA3AF);

/// Connection / authentication screen that matches the React AuthWizard
/// glassmorphism design.
///
/// Provides a full-screen gradient background with a frosted-glass card
/// containing server URL and API key fields, an amber connect button,
/// connection status indicators, error display, and flood-wait timer.
class ConnectScreen extends StatefulWidget {
  /// The connection provider to use for connecting to the server.
  final ConnectionProvider connectionProvider;

  /// Called when the connection is successfully established.
  final VoidCallback onConnected;

  const ConnectScreen({
    super.key,
    required this.connectionProvider,
    required this.onConnected,
  });

  @override
  State<ConnectScreen> createState() => _ConnectScreenState();
}

class _ConnectScreenState extends State<ConnectScreen>
    with SingleTickerProviderStateMixin {
  late final AnimationController _animController;
  late final Animation<double> _entranceAnim;

  final _serverUrlController = TextEditingController();
  final _apiKeyController = TextEditingController();
  final _formKey = GlobalKey<FormState>();

  Timer? _floodWaitTimer;
  int? _floodWaitSeconds;

  // ── Lifecycle ──────────────────────────────────────────────────────────

  @override
  void initState() {
    super.initState();
    widget.connectionProvider.addListener(_onProviderChanged);

    // If already connected, transition immediately
    if (widget.connectionProvider.isConnected) {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (mounted) widget.onConnected();
      });
    }

    _animController = AnimationController(
      vsync: this,
      duration: const Duration(milliseconds: 500),
    );
    _entranceAnim = CurvedAnimation(
      parent: _animController,
      curve: Curves.easeOutCubic,
    );
    _animController.forward();

  }

  @override
  void dispose() {
    widget.connectionProvider.removeListener(_onProviderChanged);
    _animController.dispose();
    _serverUrlController.dispose();
    _apiKeyController.dispose();
    _floodWaitTimer?.cancel();
    super.dispose();
  }

  // ── Actions ────────────────────────────────────────────────────────────

  void _handleConnect() {
    if (!_formKey.currentState!.validate()) return;

    // Clear previous flood wait on new attempt
    _floodWaitTimer?.cancel();
    _floodWaitSeconds = null;

    widget.connectionProvider.connect(
      _serverUrlController.text.trim(),
      _apiKeyController.text.trim(),
    );
  }

  void _startFloodWait(int seconds) {
    setState(() => _floodWaitSeconds = seconds);
    _floodWaitTimer?.cancel();
    _floodWaitTimer = Timer.periodic(const Duration(seconds: 1), (_) {
      if (!mounted) return;
      setState(() {
        if (_floodWaitSeconds == null || _floodWaitSeconds! <= 1) {
          _floodWaitSeconds = null;
          _floodWaitTimer?.cancel();
        } else {
          _floodWaitSeconds = _floodWaitSeconds! - 1;
        }
      });
    });
  }

  /// Called when the connection provider notifies listeners.
  /// Triggers [onConnected] when the connection succeeds and
  /// keeps the UI in sync with provider state.
  void _onProviderChanged() {
    if (widget.connectionProvider.isConnected) {
      widget.onConnected();
    }
    if (mounted) setState(() {});
  }

  /// Extracts a flood-wait duration from an error message if present.
  int? _parseFloodWait(String error) {
    // Match patterns like "429", "rate limited", "too many requests"
    final hasFloodIndicator = RegExp(
      r'(429|rate\s*limit|too\s*many\s*requests|flood)',
      caseSensitive: false,
    ).hasMatch(error);

    if (!hasFloodIndicator) return null;

    // Try to extract a numeric duration in seconds
    final durationMatch = RegExp(
      r'(?:wait|retry|after|in)\s*(\d+)\s*seconds?',
      caseSensitive: false,
    ).firstMatch(error);
    if (durationMatch != null) {
      return int.parse(durationMatch.group(1)!);
    }

    // Default 30 seconds for unquantified rate-limits
    return 30;
  }

  // ── Build ──────────────────────────────────────────────────────────────

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      backgroundColor: Colors.transparent,
      body: Container(
        decoration: const BoxDecoration(
          gradient: RadialGradient(
            center: Alignment.topLeft,
            radius: 1.2,
            colors: [_kGradientStart, _kGradientEnd],
            stops: [0.0, 1.0],
          ),
        ),
        child: Stack(
          children: [
            // ── Decorative blur circles ────────────────────────────────
            const Positioned(
              top: -100,
              left: -60,
              child: _BlurCircle(size: 500, color: _kBlurBlue),
            ),
            const Positioned(
              bottom: -80,
              right: -60,
              child: _BlurCircle(size: 400, color: _kBlurPurple),
            ),

            // ── Main content ──────────────────────────────────────────
            SafeArea(
              child: Column(
                children: [
                  // Settings gear (top right)
                  const Align(
                    alignment: Alignment.topRight,
                    child: Padding(
                      padding: EdgeInsets.only(top: 8, right: 12),
                      child: _SettingsButton(),
                    ),
                  ),
                  // Glass card
                  Expanded(
                    child: Center(
                      child: FadeTransition(
                        opacity: _entranceAnim,
                        child: ScaleTransition(
                          scale: Tween<double>(begin: 0.92, end: 1.0)
                              .animate(_entranceAnim),
                          child: SingleChildScrollView(
                            padding: const EdgeInsets.symmetric(horizontal: 24),
                    child: Builder(
                      builder: (context) {
                        final provider = widget.connectionProvider;

                        // Auto-detect flood wait from error
                        if (provider.error != null &&
                            _floodWaitSeconds == null) {
                          final parsed =
                              _parseFloodWait(provider.error!);
                          if (parsed != null) {
                            // Schedule in post-frame to avoid
                            // setState during build
                            WidgetsBinding.instance
                                .addPostFrameCallback((_) {
                              if (mounted) {
                                _startFloodWait(parsed);
                              }
                            });
                          }
                        }

                        final showFlood = _floodWaitSeconds != null;

                        return _AuthCard(
                          floodWaitSeconds:
                              showFlood ? _floodWaitSeconds : null,
                          error:
                              showFlood ? null : provider.error,
                          isLoading: provider.isLoading,
                          formKey: _formKey,
                          serverUrlController: _serverUrlController,
                          apiKeyController: _apiKeyController,
                          onConnect: _handleConnect,
                        );
                      },
                    ),
                          ),
                        ),
                      ),
                    ),
                  ),
                ],
              ),
            ),
          ],
        ),
      ),
    );
  }
}

// ═══════════════════════════════════════════════════════════════════════════
// Private widgets
// ═══════════════════════════════════════════════════════════════════════════

/// The glassmorphism card with the connection form or flood-wait display.
class _AuthCard extends StatelessWidget {
  const _AuthCard({
    required this.floodWaitSeconds,
    required this.error,
    required this.isLoading,
    required this.formKey,
    required this.serverUrlController,
    required this.apiKeyController,
    required this.onConnect,
  });

  final int? floodWaitSeconds;
  final String? error;
  final bool isLoading;
  final GlobalKey<FormState> formKey;
  final TextEditingController serverUrlController;
  final TextEditingController apiKeyController;
  final VoidCallback onConnect;

  @override
  Widget build(BuildContext context) {
    return ClipRRect(
      borderRadius: BorderRadius.circular(24),
      child: BackdropFilter(
        filter: ImageFilter.blur(sigmaX: 24, sigmaY: 24),
        child: Container(
          width: double.infinity,
          constraints: const BoxConstraints(maxWidth: 420),
          padding: const EdgeInsets.symmetric(horizontal: 32, vertical: 36),
          decoration: BoxDecoration(
            color: TelegramColors.surfaceDark.withValues(alpha: 0.85),
            borderRadius: BorderRadius.circular(24),
            border: Border.all(color: TelegramColors.borderDark),
          ),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              // ── Logo ──────────────────────────────────────────────────
              _LogoArea(),
              const SizedBox(height: 32),

              // ── Content area ──────────────────────────────────────────
              if (floodWaitSeconds != null)
                _FloodWaitDisplay(
                  seconds: floodWaitSeconds!,
                )
              else ...[
                // Form
                Form(
                  key: formKey,
                  child: Column(
                    children: [
                      _UrlField(controller: serverUrlController),
                      const SizedBox(height: 20),
                      _ApiKeyField(controller: apiKeyController),
                      const SizedBox(height: 24),
                      _ConnectButton(
                        isLoading: isLoading,
                        onTap: onConnect,
                      ),
                    ],
                  ),
                ),

                // Error
                if (error != null) ...[
                  const SizedBox(height: 16),
                  _ErrorCard(message: error!),
                ],

                // Footer help link
                const SizedBox(height: 24),
                _HelpLink(),
              ],
            ],
          ),
        ),
      ),
    );
  }
}

/// Application logo area (cloud icon + branding).
class _LogoArea extends StatelessWidget {
  @override
  Widget build(BuildContext context) {
    return const Column(
      children: [
        // Cloud icon with glow
        SizedBox(
          width: 72,
          height: 72,
          child: Icon(
            Icons.cloud_rounded,
            size: 60,
            color: Colors.white,
          ),
        ),
        SizedBox(height: 16),
        // Title
        Text(
          'Telegram Drive',
          style: TextStyle(
            fontSize: 24,
            fontWeight: FontWeight.bold,
            color: Colors.white,
            letterSpacing: -0.5,
          ),
        ),
        SizedBox(height: 4),
        // Subtitle
        Text(
          'Self-Hosted Secure Storage',
          style: TextStyle(
            fontSize: 13,
            fontWeight: FontWeight.w500,
            color: Color(0x99FFFFFF),
          ),
        ),
      ],
    );
  }
}

/// Server URL input field with globe icon.
class _UrlField extends StatelessWidget {
  const _UrlField({required this.controller});

  final TextEditingController controller;

  @override
  Widget build(BuildContext context) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        const Padding(
          padding: EdgeInsets.only(bottom: 8),
          child: Text(
            'SERVER URL',
            style: TextStyle(
              fontSize: 11,
              fontWeight: FontWeight.w600,
              color: TelegramColors.subtextDark,
              letterSpacing: 1.2,
            ),
          ),
        ),
        TextFormField(
          controller: controller,
          keyboardType: TextInputType.url,
          autocorrect: false,
          style: const TextStyle(
            color: Colors.white,
            fontSize: 14,
          ),
          decoration: InputDecoration(
            filled: true,
            fillColor: _kInputBg,
            hintText: 'http://192.168.1.100:8080',
            hintStyle: TextStyle(
              color: Colors.grey.shade700,
              fontSize: 14,
            ),
            prefixIcon: const Padding(
              padding: EdgeInsets.only(left: 16, right: 12),
              child: Icon(Icons.language_rounded, color: Colors.white, size: 20),
            ),
            prefixIconConstraints: const BoxConstraints(minWidth: 48),
            border: OutlineInputBorder(
              borderRadius: BorderRadius.circular(12),
              borderSide: BorderSide(color: TelegramColors.borderDark),
            ),
            enabledBorder: OutlineInputBorder(
              borderRadius: BorderRadius.circular(12),
              borderSide: BorderSide(color: TelegramColors.borderDark),
            ),
            focusedBorder: OutlineInputBorder(
              borderRadius: BorderRadius.circular(12),
              borderSide: const BorderSide(
                color: TelegramColors.secondaryDark,
                width: 1.5,
              ),
            ),
            errorBorder: OutlineInputBorder(
              borderRadius: BorderRadius.circular(12),
              borderSide: const BorderSide(color: _kErrorText, width: 1),
            ),
            contentPadding: const EdgeInsets.symmetric(
              horizontal: 16,
              vertical: 16,
            ),
          ),
          validator: (value) {
            if (value == null || value.trim().isEmpty) {
              return 'Server URL is required';
            }
            final trimmed = value.trim();
            if (!trimmed.startsWith('http://') &&
                !trimmed.startsWith('https://')) {
              return 'URL must start with http:// or https://';
            }
            return null;
          },
        ),
      ],
    );
  }
}

/// API Key input field with lock icon.
class _ApiKeyField extends StatelessWidget {
  const _ApiKeyField({required this.controller});

  final TextEditingController controller;

  @override
  Widget build(BuildContext context) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        const Padding(
          padding: EdgeInsets.only(bottom: 8),
          child: Text(
            'API KEY',
            style: TextStyle(
              fontSize: 11,
              fontWeight: FontWeight.w600,
              color: TelegramColors.subtextDark,
              letterSpacing: 1.2,
            ),
          ),
        ),
        TextFormField(
          controller: controller,
          obscureText: true,
          autocorrect: false,
          style: const TextStyle(
            color: Colors.white,
            fontSize: 14,
          ),
          decoration: InputDecoration(
            filled: true,
            fillColor: _kInputBg,
            hintText: 'Enter your API key',
            hintStyle: TextStyle(
              color: Colors.grey.shade700,
              fontSize: 14,
            ),
            prefixIcon: const Padding(
              padding: EdgeInsets.only(left: 16, right: 12),
              child: Icon(Icons.lock_rounded, color: Colors.white, size: 20),
            ),
            prefixIconConstraints: const BoxConstraints(minWidth: 48),
            border: OutlineInputBorder(
              borderRadius: BorderRadius.circular(12),
              borderSide: BorderSide(color: TelegramColors.borderDark),
            ),
            enabledBorder: OutlineInputBorder(
              borderRadius: BorderRadius.circular(12),
              borderSide: BorderSide(color: TelegramColors.borderDark),
            ),
            focusedBorder: OutlineInputBorder(
              borderRadius: BorderRadius.circular(12),
              borderSide: const BorderSide(
                color: TelegramColors.secondaryDark,
                width: 1.5,
              ),
            ),
            errorBorder: OutlineInputBorder(
              borderRadius: BorderRadius.circular(12),
              borderSide: const BorderSide(color: _kErrorText, width: 1),
            ),
            contentPadding: const EdgeInsets.symmetric(
              horizontal: 16,
              vertical: 16,
            ),
          ),
          validator: (value) {
            if (value == null || value.trim().isEmpty) {
              return 'API key is required';
            }
            return null;
          },
        ),
      ],
    );
  }
}

/// Amber/gold gradient connect button with loading spinner.
class _ConnectButton extends StatelessWidget {
  const _ConnectButton({required this.isLoading, required this.onTap});

  final bool isLoading;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    return SizedBox(
      width: double.infinity,
      height: 52,
      child: Material(
        borderRadius: BorderRadius.circular(12),
        color: Colors.transparent,
        child: InkWell(
          borderRadius: BorderRadius.circular(12),
          onTap: isLoading ? null : onTap,
          child: Container(
            decoration: BoxDecoration(
              gradient: const LinearGradient(
                colors: [_kAmberStart, _kAmberEnd],
                begin: Alignment.topLeft,
                end: Alignment.bottomRight,
              ),
              borderRadius: BorderRadius.circular(12),
              boxShadow: [
                BoxShadow(
                  color: _kAmberStart.withValues(alpha: 0.3),
                  blurRadius: 16,
                  offset: const Offset(0, 4),
                ),
              ],
            ),
            alignment: Alignment.center,
            child: isLoading
                ? const SizedBox(
                    width: 22,
                    height: 22,
                    child: CircularProgressIndicator(
                      strokeWidth: 2.5,
                      valueColor: AlwaysStoppedAnimation<Color>(Colors.white),
                    ),
                  )
                : const Row(
                    mainAxisAlignment: MainAxisAlignment.center,
                    children: [
                      Text(
                        'Connect',
                        style: TextStyle(
                          fontSize: 16,
                          fontWeight: FontWeight.bold,
                          color: Colors.white,
                        ),
                      ),
                      SizedBox(width: 8),
                      Icon(Icons.arrow_forward_rounded,
                          color: Colors.white, size: 20),
                    ],
                  ),
          ),
        ),
      ),
    );
  }
}

/// Error card matching the React bg-red-500/10 border-red-500/20 styling.
class _ErrorCard extends StatefulWidget {
  const _ErrorCard({required this.message});

  final String message;

  @override
  State<_ErrorCard> createState() => _ErrorCardState();
}

class _ErrorCardState extends State<_ErrorCard>
    with SingleTickerProviderStateMixin {
  late final AnimationController _ctrl;
  late final Animation<double> _fadeAnim;
  late final Animation<Offset> _slideAnim;

  @override
  void initState() {
    super.initState();
    _ctrl = AnimationController(
      vsync: this,
      duration: const Duration(milliseconds: 300),
    );
    _fadeAnim = CurvedAnimation(parent: _ctrl, curve: Curves.easeOut);
    _slideAnim = Tween<Offset>(
      begin: const Offset(0, 0.1),
      end: Offset.zero,
    ).animate(CurvedAnimation(parent: _ctrl, curve: Curves.easeOut));
    _ctrl.forward();
  }

  @override
  void dispose() {
    _ctrl.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return SlideTransition(
      position: _slideAnim,
      child: FadeTransition(
        opacity: _fadeAnim,
        child: Container(
          padding: const EdgeInsets.all(16),
          decoration: BoxDecoration(
            color: _kErrorBg,
            borderRadius: BorderRadius.circular(12),
            border: Border.all(color: _kErrorBorder),
          ),
          child: Row(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Container(
                width: 6,
                height: 6,
                margin: const EdgeInsets.only(top: 6),
                decoration: const BoxDecoration(
                  color: _kErrorText,
                  shape: BoxShape.circle,
                ),
              ),
              const SizedBox(width: 12),
              Expanded(
                child: Text(
                  widget.message,
                  style: const TextStyle(
                    color: _kErrorText,
                    fontSize: 13,
                    height: 1.4,
                  ),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

/// Flood-wait countdown display shown when rate-limited.
class _FloodWaitDisplay extends StatelessWidget {
  const _FloodWaitDisplay({required this.seconds});

  final int seconds;

  @override
  Widget build(BuildContext context) {
    final min = seconds ~/ 60;
    final sec = seconds % 60;
    final formatted =
        '${min.toStringAsFixed(0).padLeft(2, '0')}:${sec.toString().padLeft(2, '0')}';

    return Column(
      children: [
        // Hourglass icon in red-tinted circle
        Container(
          width: 64,
          height: 64,
          decoration: const BoxDecoration(
            color: _kFloodRedBg,
            shape: BoxShape.circle,
          ),
          child: const Center(
            child: Text('⏳', style: TextStyle(fontSize: 28)),
          ),
        ),
        const SizedBox(height: 24),
        const Text(
          'Too Many Requests',
          style: TextStyle(
            fontSize: 20,
            fontWeight: FontWeight.bold,
            color: Colors.white,
          ),
        ),
        const SizedBox(height: 8),
        const Text(
          'Server has temporarily limited your requests.',
          style: TextStyle(fontSize: 13, color: _kMutedGray),
        ),
        const Text(
          'Please wait before trying again.',
          style: TextStyle(fontSize: 13, color: _kMutedGray),
        ),
        const SizedBox(height: 24),
        Text(
          formatted,
          style: const TextStyle(
            fontSize: 48,
            fontFamily: 'monospace',
            color: _kFloodCountdown,
            fontWeight: FontWeight.bold,
          ),
        ),
      ],
    );
  }
}

/// "How do I get my credentials?" link at the bottom of the card.
class _HelpLink extends StatelessWidget {
  const _HelpLink();

  @override
  Widget build(BuildContext context) {
    return InkWell(
      onTap: () {
        // TODO: Open help dialog or URL
      },
      borderRadius: BorderRadius.circular(8),
      child: Padding(
        padding: const EdgeInsets.symmetric(vertical: 4, horizontal: 8),
        child: Row(
          mainAxisAlignment: MainAxisAlignment.center,
          children: [
            Icon(
              Icons.help_outline_rounded,
              size: 13,
              color: TelegramColors.secondaryDark.withValues(alpha: 0.8),
            ),
            const SizedBox(width: 6),
            Text(
              'How do I get my credentials?',
              style: TextStyle(
                fontSize: 12,
                color: TelegramColors.secondaryDark.withValues(alpha: 0.8),
              ),
            ),
          ],
        ),
      ),
    );
  }
}

/// Settings gear icon button positioned at the top-right.
class _SettingsButton extends StatelessWidget {
  const _SettingsButton();

  @override
  Widget build(BuildContext context) {
    return Material(
      color: Colors.transparent,
      child: InkWell(
        borderRadius: BorderRadius.circular(24),
        onTap: () {
          // TODO: Navigate to settings
        },
        child: Container(
          width: 44,
          height: 44,
          decoration: BoxDecoration(
            color: Colors.white.withValues(alpha: 0.1),
            borderRadius: BorderRadius.circular(24),
          ),
          child: const Icon(
            Icons.settings_rounded,
            color: Colors.white,
            size: 22,
          ),
        ),
      ),
    );
  }
}

/// Decorative blurred circle for the background glow effect.
class _BlurCircle extends StatelessWidget {
  const _BlurCircle({required this.size, required this.color});

  final double size;
  final Color color;

  @override
  Widget build(BuildContext context) {
    return Container(
      width: size,
      height: size,
      decoration: BoxDecoration(
        color: color,
        shape: BoxShape.circle,
      ),
    );
  }
}

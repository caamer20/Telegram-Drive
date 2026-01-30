import { Component, ReactNode } from 'react';
import { AlertTriangle, RefreshCw } from 'lucide-react';

interface Props {
    children: ReactNode;
}

interface State {
    hasError: boolean;
    error: Error | null;
}

export class ErrorBoundary extends Component<Props, State> {
    constructor(props: Props) {
        super(props);
        this.state = { hasError: false, error: null };
    }

    static getDerivedStateFromError(error: Error): State {
        return { hasError: true, error };
    }

    componentDidCatch(error: Error, errorInfo: React.ErrorInfo) {
        console.error('ErrorBoundary caught an error:', error, errorInfo);
    }

    handleReload = () => {
        window.location.reload();
    };

    render() {
        if (this.state.hasError) {
            return (
                <div className="h-screen w-screen flex items-center justify-center bg-telegram-bg p-8">
                    <div className="max-w-md w-full bg-telegram-surface border border-telegram-border rounded-2xl p-8 text-center shadow-2xl">
                        <div className="w-16 h-16 mx-auto mb-6 rounded-full bg-red-500/10 flex items-center justify-center">
                            <AlertTriangle className="w-8 h-8 text-red-400" />
                        </div>
                        <h1 className="text-xl font-semibold text-telegram-text mb-2">Something went wrong</h1>
                        <p className="text-telegram-subtext text-sm mb-6">
                            The application encountered an unexpected error. Please try reloading.
                        </p>

                        {this.state.error && (
                            <details className="mb-6 text-left">
                                <summary className="text-xs text-telegram-subtext cursor-pointer hover:text-telegram-text transition-colors">
                                    Technical Details
                                </summary>
                                <pre className="mt-2 p-3 bg-telegram-hover rounded-lg text-xs text-red-400 overflow-auto max-h-32">
                                    {this.state.error.message}
                                </pre>
                            </details>
                        )}

                        <button
                            onClick={this.handleReload}
                            className="inline-flex items-center gap-2 px-6 py-3 bg-telegram-primary text-black font-medium rounded-lg hover:bg-telegram-primary/90 transition-colors"
                        >
                            <RefreshCw className="w-4 h-4" />
                            Reload Application
                        </button>
                    </div>
                </div>
            );
        }

        return this.props.children;
    }
}

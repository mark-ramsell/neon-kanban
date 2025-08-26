import { useState, useCallback, useEffect } from 'react';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { Button } from '@/components/ui/button';
import { Alert, AlertDescription } from '@/components/ui/alert';
import { Separator } from '@/components/ui/separator';
import { 
  ExternalLink, 
  Loader2, 
  CheckCircle, 
  AlertCircle,
  Copy,
  ArrowRight
} from 'lucide-react';
import { jiraApi } from '@/lib/api';

interface JiraOAuthDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onSuccess: () => void;
}

interface JiraResource {
  id: string;
  name: string;
  url: string;
  scopes: string[];
  avatar_url: string;
}

type OAuthStep = 'start' | 'authorizing' | 'processing' | 'success' | 'error';

export function JiraOAuthDialog({ open, onOpenChange, onSuccess }: JiraOAuthDialogProps) {
  const [step, setStep] = useState<OAuthStep>('start');
  const [authUrl, setAuthUrl] = useState<string>('');
  const [, setState] = useState<string>('');
  const [sites, setSites] = useState<JiraResource[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [isPolling, setIsPolling] = useState(false);
  const [pollInterval, setPollInterval] = useState<ReturnType<typeof setInterval> | null>(null);
  // removed exchanged handling in same-tab flow

  const resetDialog = useCallback(() => {
    setStep('start');
    setAuthUrl('');
    setState('');
    setSites([]);
    setError(null);
    setIsPolling(false);
    if (pollInterval) {
      clearInterval(pollInterval);
      setPollInterval(null);
    }
  }, [pollInterval]);

  const startOAuth = useCallback(async () => {
    try {
      setStep('authorizing');
      setError(null);
      
      const response = await jiraApi.startOAuth(`${window.location.origin}/settings`);
      setAuthUrl(response.authorization_url);
      setState(response.state);
      
      // Navigate in the same tab so the callback lands back here
      window.location.assign(response.authorization_url);
      return;
      
    } catch (err: any) {
      console.error('Failed to start OAuth:', err);
      setError(err.message || 'Failed to start OAuth process');
      setStep('error');
    }
  }, [step]);

  const copyAuthUrl = useCallback(() => {
    if (authUrl) {
      navigator.clipboard.writeText(authUrl);
    }
  }, [authUrl]);

  const handleClose = useCallback(() => {
    resetDialog();
    onOpenChange(false);
  }, [resetDialog, onOpenChange]);

  const handleSuccess = useCallback(() => {
    onSuccess();
    resetDialog();
  }, [onSuccess, resetDialog]);

  // Cleanup on unmount
  useEffect(() => {
    return () => {
      if (pollInterval) {
        clearInterval(pollInterval);
      }
    };
  }, [pollInterval]);

  // Reset when dialog closes
  useEffect(() => {
    if (!open) {
      resetDialog();
    }
  }, [open, resetDialog]);

  const renderStartStep = () => (
    <div className="space-y-4">
      <div className="text-center space-y-2">
        <p className="text-muted-foreground">
          Connect to your Jira Cloud sites to enable task synchronization and project management integration.
        </p>
      </div>

      <Alert>
        <AlertCircle className="h-4 w-4" />
        <AlertDescription>
          You'll be redirected to Atlassian to authorize access to your Jira sites. 
          Make sure to allow access to the sites you want to integrate with Vibe Kanban.
        </AlertDescription>
      </Alert>

      <div className="flex justify-center pt-4">
        <Button onClick={startOAuth} className="flex items-center gap-2">
          <ExternalLink className="h-4 w-4" />
          Connect to Jira Cloud
        </Button>
      </div>
    </div>
  );

  const renderAuthorizingStep = () => (
    <div className="space-y-4">
      <div className="text-center space-y-4">
        <div className="flex justify-center">
          <Loader2 className="h-8 w-8 animate-spin text-blue-600" />
        </div>
        
        <div>
          <h3 className="font-medium mb-2">
            {isPolling ? 'Waiting for authorization...' : 'Opening authorization page...'}
          </h3>
          <p className="text-sm text-muted-foreground">
            {isPolling 
              ? 'Please complete the authorization in the opened window. This dialog will automatically update when complete.'
              : 'We\'re opening the Atlassian authorization page in a new window.'
            }
          </p>
        </div>
      </div>

      {authUrl && (
        <div className="space-y-2">
          <Separator />
          <div className="text-sm">
            <p className="font-medium mb-2">Having trouble? Open this URL manually:</p>
            <div className="flex items-center gap-2 p-2 bg-muted rounded text-xs font-mono break-all">
              <span className="flex-1">{authUrl}</span>
              <Button variant="ghost" size="sm" onClick={copyAuthUrl}>
                <Copy className="h-4 w-4" />
              </Button>
            </div>
          </div>
        </div>
      )}

      <div className="flex justify-center">
        <Button variant="outline" onClick={() => setStep('error')}>
          Cancel
        </Button>
      </div>
    </div>
  );

  const renderSuccessStep = () => (
    <div className="space-y-4">
      <div className="text-center space-y-4">
        <div className="flex justify-center">
          <CheckCircle className="h-8 w-8 text-green-600" />
        </div>
        
        <div>
          <h3 className="font-medium text-green-900 mb-2">
            Successfully connected to Jira Cloud!
          </h3>
          <p className="text-sm text-muted-foreground">
            You can now sync tasks and manage projects across your connected Jira sites.
          </p>
        </div>
      </div>

      {sites.length > 0 && (
        <div className="space-y-2">
          <Separator />
          <div>
            <p className="font-medium mb-2">Connected Sites:</p>
            <div className="space-y-2">
              {sites.map((site) => (
                <div key={site.id} className="flex items-center gap-2 text-sm p-2 bg-muted rounded">
                  <CheckCircle className="h-4 w-4 text-green-600" />
                  <span className="font-medium">{site.name}</span>
                  <span className="text-muted-foreground">({site.url})</span>
                </div>
              ))}
            </div>
          </div>
        </div>
      )}

      <div className="flex justify-center">
        <Button onClick={handleSuccess} className="flex items-center gap-2">
          Continue
          <ArrowRight className="h-4 w-4" />
        </Button>
      </div>
    </div>
  );

  const renderErrorStep = () => (
    <div className="space-y-4">
      <div className="text-center space-y-4">
        <div className="flex justify-center">
          <AlertCircle className="h-8 w-8 text-red-600" />
        </div>
        
        <div>
          <h3 className="font-medium text-red-900 mb-2">
            Connection Failed
          </h3>
          <p className="text-sm text-muted-foreground">
            {error || 'An error occurred during the OAuth process.'}
          </p>
        </div>
      </div>

      <div className="flex justify-center gap-2">
        <Button variant="outline" onClick={handleClose}>
          Cancel
        </Button>
        <Button onClick={() => {
          setError(null);
          startOAuth();
        }}>
          Try Again
        </Button>
      </div>
    </div>
  );

  const getStepTitle = () => {
    switch (step) {
      case 'start': return 'Connect to Jira Cloud';
      case 'authorizing': return 'Authorize Access';
      case 'processing': return 'Processing...';
      case 'success': return 'Connection Successful';
      case 'error': return 'Connection Failed';
      default: return 'Connect to Jira Cloud';
    }
  };

  const renderContent = () => {
    switch (step) {
      case 'start': return renderStartStep();
      case 'authorizing': return renderAuthorizingStep();
      case 'processing': return renderAuthorizingStep();
      case 'success': return renderSuccessStep();
      case 'error': return renderErrorStep();
      default: return renderStartStep();
    }
  };

  return (
    <Dialog open={open} onOpenChange={handleClose}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <ExternalLink className="h-5 w-5" />
            {getStepTitle()}
          </DialogTitle>
          <DialogDescription>
            Connect your Jira Cloud sites to Vibe Kanban for enhanced project management
          </DialogDescription>
        </DialogHeader>
        
        {renderContent()}
      </DialogContent>
    </Dialog>
  );
}
import { useCallback, useState, useEffect } from 'react';
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/card';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { Alert, AlertDescription } from '@/components/ui/alert';
import { Badge } from '@/components/ui/badge';
import { Separator } from '@/components/ui/separator';
import {
  ExternalLink, 
  Loader2, 
  CheckCircle, 
  XCircle, 
  Settings, 
  Trash2,
  RefreshCw
} from 'lucide-react';
import { jiraApi } from '@/lib/api';
// import { JiraOAuthDialog } from './JiraOAuthDialog';

type ConnectedSite = {
  id: string;
  name: string;
  url: string;
  scopes: string[];
  avatar_url: string;
};

interface JiraConnectionStatus {
  connected: boolean;
  site_name: string;
  user?: {
    account_id: string;
    display_name: string;
    email_address?: string;
  };
  accessible_projects: number;
  granted_scopes: string[];
}

export function JiraIntegrationCard() {
  const [connectedSites, setConnectedSites] = useState<ConnectedSite[]>([]);
  const [connectionStatuses, setConnectionStatuses] = useState<Map<string, JiraConnectionStatus>>(new Map());
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  // Same-tab OAuth flow; no dialog state needed
  const [testingConnections, setTestingConnections] = useState<Set<string>>(new Set());
  const [refreshingTokens, setRefreshingTokens] = useState<Set<string>>(new Set());
  const [clientId, setClientId] = useState('');
  const [clientSecret, setClientSecret] = useState('');
  const [credsConfigured, setCredsConfigured] = useState<boolean | null>(null);
  const [savingCreds, setSavingCreds] = useState(false);

  const isValidStatus = (value: unknown): value is JiraConnectionStatus => {
    return (
      !!value &&
      typeof value === 'object' &&
      'connected' in (value as any) &&
      'accessible_projects' in (value as any)
    );
  };

  const loadConfigs = useCallback(async () => {
    try {
      setLoading(true);
      setError(null);
      // Load credential status first
      try {
        const status = await jiraApi.getCredentialsStatus();
        setCredsConfigured(status.configured);
      } catch (e) {
        // Non-fatal for the rest of UI
      }
      const sites = await jiraApi.getAccessibleSites();
      // Normalize any backend field name differences
      const normalized = sites.map((s) => ({
        ...s,
        avatar_url: (s as any).avatar_url ?? (s as any).avatarUrl ?? '',
      }));
      setConnectedSites(normalized);
    } catch (err: any) {
      console.error('Failed to load Jira sites:', err);
      setError('Failed to load Jira sites');
    } finally {
      setLoading(false);
    }
  }, []);

  const testConnection = useCallback(async (cloudid: string) => {
    try {
      setTestingConnections(prev => new Set([...prev, cloudid]));
      const status = await jiraApi.testConnection(cloudid);
      const normalized: JiraConnectionStatus = isValidStatus(status)
        ? status
        : {
            connected: false,
            site_name: 'Unknown',
            accessible_projects: 0,
            granted_scopes: [],
          };
      setConnectionStatuses(prev => new Map([...prev, [cloudid, normalized]]));
    } catch (err: any) {
      console.error('Failed to test connection:', err);
      setConnectionStatuses(prev => new Map([...prev, [cloudid, {
        connected: false,
        site_name: 'Connection Failed',
        accessible_projects: 0,
        granted_scopes: [],
      }]]));
    } finally {
      setTestingConnections(prev => {
        const newSet = new Set(prev);
        newSet.delete(cloudid);
        return newSet;
      });
    }
  }, []);

  const refreshToken = useCallback(async (cloudid: string) => {
    try {
      setRefreshingTokens(prev => new Set([...prev, cloudid]));
      await jiraApi.refreshToken(cloudid);
      // Test connection after refresh to update status
      await testConnection(cloudid);
    } catch (err: any) {
      console.error('Failed to refresh token:', err);
      setError(`Failed to refresh token for ${cloudid}`);
    } finally {
      setRefreshingTokens(prev => {
        const newSet = new Set(prev);
        newSet.delete(cloudid);
        return newSet;
      });
    }
  }, [testConnection]);

  const disconnectSite = useCallback(async (cloudid: string, siteName: string) => {
    if (!confirm(`Are you sure you want to disconnect from "${siteName}"?`)) {
      return;
    }

    try {
      await jiraApi.revokeAccess(cloudid);
      setConnectedSites(prev => prev.filter(site => site.id !== cloudid));
      setConnectionStatuses(prev => {
        const newMap = new Map(prev);
        newMap.delete(cloudid);
        return newMap;
      });
    } catch (err: any) {
      console.error('Failed to disconnect site:', err);
      setError(`Failed to disconnect from ${siteName}`);
    }
  }, []);

  const handleStartOAuth = useCallback(async () => {
    try {
      const resp = await jiraApi.startOAuth(`${window.location.origin}/settings`);
      window.location.assign(resp.authorization_url);
    } catch (e) {
      setError('Failed to start OAuth');
    }
  }, []);

  // Load configs on mount
  useEffect(() => {
    loadConfigs();
  }, [loadConfigs]);

  // Test connections for all sites on load (use their id as cloudid)
  useEffect(() => {
    if (connectedSites.length > 0) {
      connectedSites.forEach(site => {
        testConnection(site.id);
      });
    }
  }, [connectedSites, testConnection]);

  if (loading) {
    return (
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <ExternalLink className="h-5 w-5" />
            Jira Cloud Integration
          </CardTitle>
          <CardDescription>
            Connect to your Jira Cloud sites to sync tasks
          </CardDescription>
        </CardHeader>
        <CardContent className="flex items-center justify-center p-8">
          <Loader2 className="h-6 w-6 animate-spin" />
          <span className="ml-2">Loading configurations...</span>
        </CardContent>
      </Card>
    );
  }

  return (
    <>
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <ExternalLink className="h-5 w-5" />
            Jira Cloud Integration
          </CardTitle>
          <CardDescription>
            Connect to your Jira Cloud sites to sync tasks and track progress
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          {error && (
            <Alert variant="destructive">
              <AlertDescription>{error}</AlertDescription>
            </Alert>
          )}

          <div className="space-y-3 p-3 border rounded-md">
            <div className="flex items-center justify-between">
              <h4 className="font-medium">OAuth Credentials</h4>
              {credsConfigured === true && (
                <Badge variant="secondary">Configured</Badge>
              )}
              {credsConfigured === false && (
                <Badge variant="destructive">Not configured</Badge>
              )}
            </div>
            <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
              <div className="space-y-1.5">
                <Label htmlFor="jira-client-id">Client ID</Label>
                <Input
                  id="jira-client-id"
                  type="password"
                  placeholder="Enter Jira OAuth client ID"
                  value={clientId}
                  onChange={(e) => setClientId(e.target.value)}
                />
              </div>
              <div className="space-y-1.5">
                <Label htmlFor="jira-client-secret">Client Secret</Label>
                <Input
                  id="jira-client-secret"
                  type="password"
                  placeholder="Enter Jira OAuth client secret"
                  value={clientSecret}
                  onChange={(e) => setClientSecret(e.target.value)}
                />
              </div>
            </div>
            <div className="space-y-1.5">
              <Label>Redirect URI (add this to your Atlassian App)</Label>
              <Input
                readOnly
                value={`${window.location.origin}/settings`}
              />
            </div>
            <div className="flex items-center gap-2">
              <Button
                onClick={async () => {
                  setSavingCreds(true);
                  setError(null);
                  try {
                    await jiraApi.setCredentials(clientId, clientSecret);
                    setClientId('');
                    setClientSecret('');
                    setCredsConfigured(true);
                  } catch (e: any) {
                    setError(e?.message || 'Failed to save credentials');
                  } finally {
                    setSavingCreds(false);
                  }
                }}
                disabled={savingCreds || !clientId || !clientSecret}
              >
                {savingCreds ? 'Saving…' : 'Save OAuth Credentials'}
              </Button>
              {credsConfigured && (
                <Button
                  variant="outline"
                  onClick={async () => {
                    setSavingCreds(true);
                    setError(null);
                    try {
                      await jiraApi.deleteCredentials();
                      setCredsConfigured(false);
                    } catch (e: any) {
                      setError(e?.message || 'Failed to clear credentials');
                    } finally {
                      setSavingCreds(false);
                    }
                  }}
                >
                  Clear Credentials
                </Button>
              )}
            </div>
          </div>

          {connectedSites.length === 0 ? (
            <div className="text-center py-8">
              <p className="text-muted-foreground mb-4">
                No Jira sites connected yet
              </p>
              <Button onClick={handleStartOAuth}>
                Connect to Jira Cloud
              </Button>
            </div>
          ) : (
            <div className="space-y-4">
              {connectedSites.map((site) => {
                const rawStatus = connectionStatuses.get(site.id);
                const status = isValidStatus(rawStatus) ? rawStatus : undefined;
                const isTesting = testingConnections.has(site.id);
                const isRefreshing = refreshingTokens.has(site.id);

                return (
                  <div key={site.id} className="border rounded-lg p-4">
                    <div className="flex items-start justify-between">
                      <div className="flex-1 min-w-0">
                        <div className="flex items-center gap-2 mb-2">
                          <h4 className="font-medium truncate">{site.name}</h4>
                          {status ? (
                            status.connected ? (
                              <CheckCircle className="h-4 w-4 text-green-600" />
                            ) : (
                              <XCircle className="h-4 w-4 text-red-600" />
                            )
                          ) : isTesting ? (
                            <Loader2 className="h-4 w-4 animate-spin" />
                          ) : null}
                        </div>
                        
                        <p className="text-sm text-muted-foreground mb-2">
                          {site.url}
                        </p>

                        {status && (
                          <div className="space-y-2">
                            {status.user && (
                              <p className="text-sm">
                                <span className="text-muted-foreground">Signed in as:</span>{' '}
                                <span className="font-medium">{status.user.display_name}</span>
                                {status.user.email_address && (
                                  <span className="text-muted-foreground ml-1">
                                    ({status.user.email_address})
                                  </span>
                                )}
                              </p>
                            )}
                            
                            <div className="flex items-center gap-4 text-sm text-muted-foreground">
                              <span>
                                {status.accessible_projects} project{status.accessible_projects !== 1 ? 's' : ''}
                              </span>
                              <Separator orientation="vertical" className="h-4" />
                              {Array.isArray(status.granted_scopes) && status.granted_scopes.length > 0 && (
                                <div className="flex flex-wrap gap-1">
                                  {status.granted_scopes.map((scope) => (
                                    <Badge key={scope} variant="secondary" className="text-xs">
                                      {scope}
                                    </Badge>
                                  ))}
                                </div>
                              )}
                            </div>
                          </div>
                        )}
                      </div>

                      <div className="flex items-center gap-2 ml-4">
                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={() => testConnection(site.id)}
                          disabled={isTesting}
                        >
                          {isTesting ? (
                            <Loader2 className="h-4 w-4 animate-spin" />
                          ) : (
                            <Settings className="h-4 w-4" />
                          )}
                        </Button>

                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={() => refreshToken(site.id)}
                          disabled={isRefreshing}
                          title="Refresh access token"
                        >
                          {isRefreshing ? (
                            <Loader2 className="h-4 w-4 animate-spin" />
                          ) : (
                            <RefreshCw className="h-4 w-4" />
                          )}
                        </Button>

                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={() => disconnectSite(site.id, site.name)}
                        >
                          <Trash2 className="h-4 w-4" />
                        </Button>
                      </div>
                    </div>
                  </div>
                );
              })}

              <Separator />

              <Button 
                variant="outline" 
                onClick={handleStartOAuth}
                className="w-full"
              >
                Connect Another Site
              </Button>
            </div>
          )}
        </CardContent>
      </Card>

      {/* Dialog removed for same-tab OAuth flow */}
    </>
  );
}
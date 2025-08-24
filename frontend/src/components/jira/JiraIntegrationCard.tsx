import { useCallback, useState, useEffect } from 'react';
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/card';
import { Button } from '@/components/ui/button';
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
import { JiraOAuthDialog } from './JiraOAuthDialog';

interface JiraResource {
  id: string;
  name: string;
  url: string;
  scopes: string[];
  avatar_url: string;
}

interface JiraConfig {
  id: string;
  cloudid: string;
  site_name: string;
  site_url: string;
  is_active: boolean;
}

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
  const [connectedSites, setConnectedSites] = useState<JiraConfig[]>([]);
  const [connectionStatuses, setConnectionStatuses] = useState<Map<string, JiraConnectionStatus>>(new Map());
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [showOAuthDialog, setShowOAuthDialog] = useState(false);
  const [testingConnections, setTestingConnections] = useState<Set<string>>(new Set());
  const [refreshingTokens, setRefreshingTokens] = useState<Set<string>>(new Set());

  const loadConfigs = useCallback(async () => {
    try {
      setLoading(true);
      setError(null);
      const configs = await jiraApi.getConfigs();
      setConnectedSites(configs);
    } catch (err: any) {
      console.error('Failed to load Jira configs:', err);
      setError('Failed to load Jira configurations');
    } finally {
      setLoading(false);
    }
  }, []);

  const testConnection = useCallback(async (cloudid: string) => {
    try {
      setTestingConnections(prev => new Set([...prev, cloudid]));
      const status = await jiraApi.testConnection(cloudid);
      setConnectionStatuses(prev => new Map([...prev, [cloudid, status]]));
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
      setConnectedSites(prev => prev.filter(site => site.cloudid !== cloudid));
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

  const handleOAuthSuccess = useCallback(async () => {
    setShowOAuthDialog(false);
    await loadConfigs();
  }, [loadConfigs]);

  // Load configs on mount
  useEffect(() => {
    loadConfigs();
  }, [loadConfigs]);

  // Test connections for all sites on load
  useEffect(() => {
    if (connectedSites.length > 0) {
      connectedSites.forEach(site => {
        testConnection(site.cloudid);
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

          {connectedSites.length === 0 ? (
            <div className="text-center py-8">
              <p className="text-muted-foreground mb-4">
                No Jira sites connected yet
              </p>
              <Button onClick={() => setShowOAuthDialog(true)}>
                Connect to Jira Cloud
              </Button>
            </div>
          ) : (
            <div className="space-y-4">
              {connectedSites.map((site) => {
                const status = connectionStatuses.get(site.cloudid);
                const isTesting = testingConnections.has(site.cloudid);
                const isRefreshing = refreshingTokens.has(site.cloudid);

                return (
                  <div key={site.cloudid} className="border rounded-lg p-4">
                    <div className="flex items-start justify-between">
                      <div className="flex-1 min-w-0">
                        <div className="flex items-center gap-2 mb-2">
                          <h4 className="font-medium truncate">{site.site_name}</h4>
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
                          {site.site_url}
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
                              <div className="flex flex-wrap gap-1">
                                {status.granted_scopes.map((scope) => (
                                  <Badge key={scope} variant="secondary" className="text-xs">
                                    {scope}
                                  </Badge>
                                ))}
                              </div>
                            </div>
                          </div>
                        )}
                      </div>

                      <div className="flex items-center gap-2 ml-4">
                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={() => testConnection(site.cloudid)}
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
                          onClick={() => refreshToken(site.cloudid)}
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
                          onClick={() => disconnectSite(site.cloudid, site.site_name)}
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
                onClick={() => setShowOAuthDialog(true)}
                className="w-full"
              >
                Connect Another Site
              </Button>
            </div>
          )}
        </CardContent>
      </Card>

      <JiraOAuthDialog
        open={showOAuthDialog}
        onOpenChange={setShowOAuthDialog}
        onSuccess={handleOAuthSuccess}
      />
    </>
  );
}